use std::{
    collections::{HashMap, HashSet, VecDeque},
    mem::size_of,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use image::{DynamicImage, imageops::FilterType};
use ratatui::layout::Size;
use ratatui_image::{
    FontSize, Resize,
    picker::{Picker, ProtocolType},
    protocol::{Protocol, halfblocks::Halfblocks},
};

use crate::{
    app::{EntryId, SourceRevision},
    kitty::{KittyProtocol, KittySession},
    persistent_cache::PersistentCache,
    settings::ThumbnailQuality,
};

const MAX_CACHE_ENTRIES: usize = 256;
const MAX_CACHE_BYTES: u64 = 128 * 1024 * 1024;
const MAX_RAW_THUMBNAIL_BYTES: u64 = 64 * 1024 * 1024;
const MAX_PENDING_REQUESTS: usize = 64;
/// Backoff before a failed key may be requested again; after the last entry
/// the failure is permanent until the key identity changes.
const FAILURE_BACKOFF: [Duration; 2] = [Duration::from_secs(2), Duration::from_secs(8)];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct ThumbnailKey {
    pub(crate) entry_id: EntryId,
    pub(crate) revision: SourceRevision,
    pub(crate) size: Size,
    pub(crate) quality: ThumbnailQuality,
}

pub(crate) enum PreparedThumbnail {
    Standard(Protocol),
    Kitty(KittyProtocol),
}

pub(crate) enum ThumbnailDisplay<'a> {
    Loading,
    Ready(&'a PreparedThumbnail),
    Error(&'a str),
}

impl PreparedThumbnail {
    #[cfg(test)]
    pub(crate) fn size(&self) -> Size {
        match self {
            Self::Standard(protocol) => protocol.size(),
            Self::Kitty(protocol) => protocol.size(),
        }
    }

    pub(crate) fn resident_bytes(&self) -> u64 {
        match self {
            Self::Kitty(protocol) => protocol.resident_bytes(),
            Self::Standard(standard @ Protocol::Halfblocks(_)) => u64::from(standard.size().width)
                .saturating_mul(u64::from(standard.size().height))
                .saturating_mul(size_of::<ratatui_image::protocol::halfblocks::HalfBlock>() as u64),
            Self::Standard(Protocol::Sixel(protocol)) => protocol.data.capacity() as u64,
            Self::Standard(Protocol::ITerm2(protocol)) => protocol.data.capacity() as u64,
            Self::Standard(Protocol::Kitty(_)) => u64::MAX,
        }
    }

    pub(crate) fn rearm(&self) -> bool {
        match self {
            Self::Kitty(protocol) => protocol.rearm(),
            Self::Standard(_) => false,
        }
    }
}

struct Request {
    key: ThumbnailKey,
    path: Arc<Path>,
    generation: u64,
}

struct Response {
    key: ThumbnailKey,
    protocol: Option<Result<PreparedProtocol, String>>,
}

struct PreparedProtocol {
    protocol: PreparedThumbnail,
    disk_hit: bool,
}

struct Failure {
    message: String,
    attempts: u8,
    retry_at: Option<Instant>,
}

impl Failure {
    fn may_retry(&self, now: Instant) -> bool {
        self.retry_at.is_some_and(|retry_at| now >= retry_at)
    }
}

pub(crate) struct Thumbnails {
    request_sender: mpsc::SyncSender<Request>,
    response_receiver: mpsc::Receiver<Response>,
    cache: HashMap<ThumbnailKey, PreparedThumbnail>,
    insertion_order: VecDeque<ThumbnailKey>,
    cache_bytes: u64,
    cache_entry_limit: usize,
    cache_byte_limit: u64,
    visible: HashSet<ThumbnailKey>,
    pending: HashSet<ThumbnailKey>,
    deferred: HashSet<ThumbnailKey>,
    failed: HashMap<ThumbnailKey, Failure>,
    failure_order: VecDeque<ThumbnailKey>,
    generation: Arc<AtomicU64>,
    persistent_hits: usize,
    persistent_misses: usize,
    response_closed: bool,
}

impl Thumbnails {
    #[cfg(test)]
    pub(crate) fn new(picker: Picker) -> Self {
        Self::with_session(picker, KittySession::from_process())
    }

    pub(crate) fn with_session(picker: Picker, kitty_session: KittySession) -> Self {
        Self::with_limits(
            picker,
            kitty_session,
            MAX_CACHE_ENTRIES,
            MAX_CACHE_BYTES,
            MAX_PENDING_REQUESTS,
        )
    }

    fn with_limits(
        picker: Picker,
        kitty_session: KittySession,
        cache_entry_limit: usize,
        cache_byte_limit: u64,
        request_capacity: usize,
    ) -> Self {
        let worker_count = thread::available_parallelism()
            .map(|count| count.get().saturating_sub(1).clamp(1, 8))
            .unwrap_or(1);
        let (request_sender, request_receiver) = mpsc::sync_channel(request_capacity);
        let (response_sender, response_receiver) = mpsc::sync_channel(worker_count * 2);
        let request_receiver = Arc::new(Mutex::new(request_receiver));
        let generation = Arc::new(AtomicU64::new(0));
        let persistent_cache = PersistentCache::discover();

        for index in 0..worker_count {
            let requests = Arc::clone(&request_receiver);
            let responses = response_sender.clone();
            let picker = picker.clone();
            let generation = Arc::clone(&generation);
            let persistent_cache = persistent_cache.clone();
            let kitty_session = kitty_session.clone();
            thread::Builder::new()
                .name(format!("red-table-thumbnail-{index}"))
                .spawn(move || {
                    worker(
                        requests,
                        responses,
                        picker,
                        persistent_cache,
                        kitty_session,
                        generation,
                    );
                })
                .expect("thumbnail worker should start");
        }
        drop(response_sender);

        Self {
            request_sender,
            response_receiver,
            cache: HashMap::new(),
            insertion_order: VecDeque::new(),
            cache_bytes: 0,
            cache_entry_limit,
            cache_byte_limit,
            visible: HashSet::new(),
            pending: HashSet::new(),
            deferred: HashSet::new(),
            failed: HashMap::new(),
            failure_order: VecDeque::new(),
            generation,
            persistent_hits: 0,
            persistent_misses: 0,
            response_closed: false,
        }
    }

    pub(crate) fn display(&mut self, key: ThumbnailKey) -> ThumbnailDisplay<'_> {
        let ready_key = if self.cache.contains_key(&key) {
            Some(key)
        } else if self.failed.contains_key(&key) {
            None
        } else if self.pending.contains(&key) || self.deferred.contains(&key) {
            self.compatible_key(key)
        } else {
            None
        };
        if let Some(ready_key) = ready_key {
            self.touch(ready_key);
            return ThumbnailDisplay::Ready(
                self.cache
                    .get(&ready_key)
                    .expect("a touched thumbnail key must remain cached"),
            );
        }
        self.failed
            .get(&key)
            .map_or(ThumbnailDisplay::Loading, |failure| {
                ThumbnailDisplay::Error(&failure.message)
            })
    }

    #[cfg(test)]
    pub(crate) fn get_for_display(&mut self, key: ThumbnailKey) -> Option<&PreparedThumbnail> {
        let ready_key = self.cache.contains_key(&key).then_some(key).or_else(|| {
            self.pending
                .contains(&key)
                .then(|| self.compatible_key(key))
                .flatten()
        })?;
        self.touch(ready_key);
        self.cache.get(&ready_key)
    }

    pub(crate) fn begin_generation(&mut self) -> u64 {
        self.deferred.clear();
        self.generation.fetch_add(1, Ordering::AcqRel) + 1
    }

    pub(crate) fn request(&mut self, key: ThumbnailKey, path: Arc<Path>, generation: u64) -> bool {
        self.request_at(key, path, generation, Instant::now())
    }

    fn request_at(
        &mut self,
        key: ThumbnailKey,
        path: Arc<Path>,
        generation: u64,
        now: Instant,
    ) -> bool {
        if key.size.width == 0
            || key.size.height == 0
            || self.cache.contains_key(&key)
            || self.pending.contains(&key)
            || self
                .failed
                .get(&key)
                .is_some_and(|failure| !failure.may_retry(now))
        {
            return false;
        }

        if self.response_closed {
            self.insert_failure(key, "thumbnail workers are unavailable".into());
            return true;
        }

        match self.request_sender.try_send(Request {
            key,
            path,
            generation,
        }) {
            Ok(()) => {
                self.deferred.remove(&key);
                self.pending.insert(key);
                true
            }
            Err(mpsc::TrySendError::Full(_)) => {
                self.deferred.insert(key);
                false
            }
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.response_closed = true;
                self.insert_failure(key, "thumbnail workers are unavailable".into());
                true
            }
        }
    }

    pub(crate) fn drain(&mut self) -> (usize, usize) {
        let mut completed = 0;
        let mut errors = 0;
        loop {
            match self.response_receiver.try_recv() {
                Ok(response) => {
                    completed += 1;
                    self.pending.remove(&response.key);
                    match response.protocol {
                        Some(Ok(prepared)) => {
                            if prepared.disk_hit {
                                self.persistent_hits += 1;
                            } else {
                                self.persistent_misses += 1;
                            }
                            self.insert(response.key, prepared.protocol);
                        }
                        Some(Err(error)) => {
                            self.insert_failure(response.key, error);
                            errors += 1;
                        }
                        None => {}
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !self.response_closed {
                        self.response_closed = true;
                        let pending = self.pending.drain().collect::<Vec<_>>();
                        errors += pending.len();
                        for key in pending {
                            self.insert_failure(
                                key,
                                "thumbnail workers stopped unexpectedly".into(),
                            );
                        }
                    }
                    break;
                }
            }
        }
        (completed, errors)
    }

    pub(crate) fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub(crate) const fn persistent_stats(&self) -> (usize, usize) {
        (self.persistent_hits, self.persistent_misses)
    }

    pub(crate) fn cache_stats(&self) -> (usize, u64) {
        (self.cache.len(), self.cache_bytes)
    }

    pub(crate) fn update_visible(&mut self, keys: impl IntoIterator<Item = ThumbnailKey>) -> bool {
        let next = keys.into_iter().collect::<HashSet<_>>();
        let mut rearmed = false;
        for key in next.difference(&self.visible) {
            if let Some(prepared) = self.cache.get(key).or_else(|| self.compatible_cached(*key)) {
                rearmed |= prepared.rearm();
            }
        }
        self.visible = next;
        rearmed
    }

    fn compatible_cached(&self, key: ThumbnailKey) -> Option<&PreparedThumbnail> {
        self.compatible_key(key)
            .and_then(|candidate| self.cache.get(&candidate))
    }

    fn compatible_key(&self, key: ThumbnailKey) -> Option<ThumbnailKey> {
        self.insertion_order
            .iter()
            .rev()
            .copied()
            .find(|candidate| {
                candidate.entry_id == key.entry_id
                    && candidate.revision == key.revision
                    && candidate.size == key.size
                    && self.cache.contains_key(candidate)
            })
    }

    fn touch(&mut self, key: ThumbnailKey) {
        self.insertion_order.retain(|candidate| *candidate != key);
        self.insertion_order.push_back(key);
    }

    pub(crate) fn insert(&mut self, key: ThumbnailKey, protocol: PreparedThumbnail) {
        let resident_bytes = protocol.resident_bytes();
        if resident_bytes > self.cache_byte_limit {
            self.insert_failure(
                key,
                format!(
                    "prepared thumbnail requires {} MiB, above the {} MiB memory limit",
                    resident_bytes.div_ceil(1024 * 1024),
                    self.cache_byte_limit / (1024 * 1024)
                ),
            );
            return;
        }
        self.failed.remove(&key);
        self.failure_order.retain(|candidate| *candidate != key);
        if let Some(previous) = self.cache.remove(&key) {
            self.cache_bytes = self.cache_bytes.saturating_sub(previous.resident_bytes());
            self.insertion_order.retain(|candidate| *candidate != key);
        }
        while self.cache.len() >= self.cache_entry_limit
            || self.cache_bytes.saturating_add(resident_bytes) > self.cache_byte_limit
        {
            if let Some(oldest) = self.insertion_order.pop_front() {
                if let Some(removed) = self.cache.remove(&oldest) {
                    self.cache_bytes = self.cache_bytes.saturating_sub(removed.resident_bytes());
                }
            } else {
                break;
            }
        }
        self.cache.insert(key, protocol);
        self.insertion_order.push_back(key);
        self.cache_bytes += resident_bytes;
    }

    fn insert_failure(&mut self, key: ThumbnailKey, error: String) {
        self.insert_failure_at(key, error, Instant::now());
    }

    fn insert_failure_at(&mut self, key: ThumbnailKey, error: String, now: Instant) {
        if let Some(existing) = self.failed.get_mut(&key) {
            existing.attempts = existing.attempts.saturating_add(1);
            existing.retry_at = retry_deadline(existing.attempts, now);
            existing.message = error;
            return;
        }
        while self.failed.len() >= self.cache_entry_limit {
            let Some(oldest) = self.failure_order.pop_front() else {
                break;
            };
            self.failed.remove(&oldest);
        }
        self.failed.insert(
            key,
            Failure {
                message: error,
                attempts: 1,
                retry_at: retry_deadline(1, now),
            },
        );
        self.failure_order.push_back(key);
    }
}

fn retry_deadline(attempts: u8, now: Instant) -> Option<Instant> {
    FAILURE_BACKOFF
        .get(usize::from(attempts.saturating_sub(1)))
        .map(|backoff| now + *backoff)
}

#[cfg(test)]
impl Thumbnails {
    fn failure_attempts(&self, key: ThumbnailKey) -> Option<u8> {
        self.failed.get(&key).map(|failure| failure.attempts)
    }
}

fn worker(
    receiver: Arc<Mutex<mpsc::Receiver<Request>>>,
    sender: mpsc::SyncSender<Response>,
    picker: Picker,
    persistent_cache: PersistentCache,
    kitty_session: KittySession,
    current_generation: Arc<AtomicU64>,
) {
    loop {
        let request = match receiver.lock() {
            Ok(receiver) => receiver.recv(),
            Err(_) => return,
        };
        let Ok(request) = request else {
            return;
        };

        let protocol = if request.generation == current_generation.load(Ordering::Acquire) {
            let result = prepare(
                &picker,
                &persistent_cache,
                &request.path,
                request.key.size,
                request.key.quality,
                &kitty_session,
            );
            (request.generation == current_generation.load(Ordering::Acquire)).then_some(result)
        } else {
            None
        };

        if sender
            .send(Response {
                key: request.key,
                protocol,
            })
            .is_err()
        {
            return;
        }
    }
}

#[derive(Clone, Copy)]
struct QualityProfile {
    filter: FilterType,
    unsharpen: Option<(f32, i32)>,
}

fn quality_profile(quality: ThumbnailQuality) -> QualityProfile {
    match quality.level() {
        1 => QualityProfile {
            filter: FilterType::Nearest,
            unsharpen: None,
        },
        2 => QualityProfile {
            filter: FilterType::Triangle,
            unsharpen: None,
        },
        3 => QualityProfile {
            filter: FilterType::CatmullRom,
            unsharpen: None,
        },
        4 => QualityProfile {
            filter: FilterType::Lanczos3,
            unsharpen: None,
        },
        5 => QualityProfile {
            filter: FilterType::Lanczos3,
            unsharpen: Some((0.4, 6)),
        },
        6 => QualityProfile {
            filter: FilterType::Lanczos3,
            unsharpen: Some((0.6, 4)),
        },
        7 => QualityProfile {
            filter: FilterType::Lanczos3,
            unsharpen: Some((0.8, 3)),
        },
        8 => QualityProfile {
            filter: FilterType::Lanczos3,
            unsharpen: Some((1.0, 2)),
        },
        9 => QualityProfile {
            filter: FilterType::Lanczos3,
            unsharpen: Some((1.2, 1)),
        },
        _ => unreachable!("ThumbnailQuality guarantees levels from 1 to 9"),
    }
}

fn prepare(
    picker: &Picker,
    persistent_cache: &PersistentCache,
    path: &Path,
    size: Size,
    quality: ThumbnailQuality,
    kitty_session: &KittySession,
) -> Result<PreparedProtocol, String> {
    let required_pixels = required_source_pixels(picker, size);
    let source = persistent_cache.prepare(path, required_pixels)?;
    let protocol = prepare_image_with_session(picker, source.image, size, quality, kitty_session)?;
    let resident_bytes = protocol.resident_bytes();
    if resident_bytes > MAX_CACHE_BYTES {
        return Err(format!(
            "prepared thumbnail requires {} MiB, above the {} MiB memory limit",
            resident_bytes.div_ceil(1024 * 1024),
            MAX_CACHE_BYTES / (1024 * 1024)
        ));
    }
    Ok(PreparedProtocol {
        protocol,
        disk_hit: source.disk_hit,
    })
}

fn required_source_pixels(picker: &Picker, size: Size) -> (u32, u32) {
    if picker.protocol_type() == ProtocolType::Halfblocks {
        (u32::from(size.width), u32::from(size.height) * 2)
    } else {
        let font = picker.font_size();
        (
            u32::from(size.width) * u32::from(font.width),
            u32::from(size.height) * u32::from(font.height),
        )
    }
}

#[cfg(test)]
pub(crate) fn prepare_image(
    picker: &Picker,
    image: DynamicImage,
    size: Size,
    quality: ThumbnailQuality,
) -> Result<PreparedThumbnail, String> {
    prepare_image_with_session(picker, image, size, quality, &KittySession::from_process())
}

fn prepare_image_with_session(
    picker: &Picker,
    image: DynamicImage,
    size: Size,
    quality: ThumbnailQuality,
    kitty_session: &KittySession,
) -> Result<PreparedThumbnail, String> {
    let (image, encoded_size) = resize_for_protocol(picker, image, size, quality)?;
    if picker.protocol_type() == ProtocolType::Halfblocks {
        return Halfblocks::new(image, encoded_size)
            .map(Protocol::Halfblocks)
            .map(PreparedThumbnail::Standard)
            .map_err(|error| error.to_string());
    }
    if picker.protocol_type() == ProtocolType::Kitty {
        return KittyProtocol::new(image, encoded_size, kitty_session.clone())
            .map(PreparedThumbnail::Kitty);
    }
    picker
        .new_protocol(image, encoded_size, Resize::Fit(None))
        .map(PreparedThumbnail::Standard)
        .map_err(|error| error.to_string())
}

pub(crate) fn encode_exact(
    picker: &Picker,
    image: DynamicImage,
    size: Size,
    kitty_session: &KittySession,
) -> Result<PreparedThumbnail, String> {
    if picker.protocol_type() == ProtocolType::Halfblocks {
        return Halfblocks::new(image, size)
            .map(Protocol::Halfblocks)
            .map(PreparedThumbnail::Standard)
            .map_err(|error| error.to_string());
    }
    if picker.protocol_type() == ProtocolType::Kitty {
        return KittyProtocol::new(image, size, kitty_session.clone())
            .map(PreparedThumbnail::Kitty);
    }
    picker
        .new_protocol(image, size, Resize::Fit(None))
        .map(PreparedThumbnail::Standard)
        .map_err(|error| error.to_string())
}

fn resize_for_protocol(
    picker: &Picker,
    image: DynamicImage,
    size: Size,
    quality: ThumbnailQuality,
) -> Result<(DynamicImage, Size), String> {
    let profile = quality_profile(quality);
    let halfblocks = picker.protocol_type() == ProtocolType::Halfblocks;
    let resize = if halfblocks {
        Resize::Scale(Some(profile.filter))
    } else {
        Resize::Fit(Some(profile.filter))
    };
    let font_size = if halfblocks {
        FontSize::new(1, 2)
    } else {
        picker.font_size()
    };
    let encoded_size = resize.size_for(&image, font_size, size);
    let sample_width = u64::from(encoded_size.width) * u64::from(font_size.width);
    let sample_height = u64::from(encoded_size.height) * u64::from(font_size.height);
    let raw_bytes = sample_width.saturating_mul(sample_height).saturating_mul(4);
    if raw_bytes > MAX_RAW_THUMBNAIL_BYTES {
        return Err(format!(
            "thumbnail target requires {} MiB of RGBA data, above the {} MiB limit",
            raw_bytes.div_ceil(1024 * 1024),
            MAX_RAW_THUMBNAIL_BYTES / (1024 * 1024)
        ));
    }
    let mut image = if halfblocks {
        image.resize_exact(
            u32::from(encoded_size.width),
            u32::from(encoded_size.height) * 2,
            profile.filter,
        )
    } else {
        resize.resize(&image, picker.font_size(), encoded_size, None)
    };
    if let Some((sigma, threshold)) = profile.unsharpen {
        image = image.unsharpen(sigma, threshold);
    }
    Ok((image, encoded_size))
}

#[cfg(test)]
mod tests {
    use std::{
        fs, thread,
        time::{Duration, Instant, SystemTime},
    };

    use image::{Rgb, RgbImage};
    use ratatui::{Terminal, backend::TestBackend};
    use ratatui_image::Image;

    use super::*;

    fn temporary_path(extension: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "red-table-thumbnail-{}-{nonce}.{extension}",
            std::process::id()
        ))
    }

    #[test]
    fn prepares_a_bounded_thumbnail_from_a_real_image() {
        let path = temporary_path("png");
        RgbImage::from_pixel(64, 32, Rgb([180, 20, 30]))
            .save(&path)
            .unwrap();

        let protocol = prepare(
            &Picker::halfblocks(),
            &PersistentCache::disabled(),
            &path,
            Size::new(12, 6),
            "7".parse().unwrap(),
            &KittySession::with_tmux(false),
        )
        .unwrap();
        assert!(protocol.protocol.size().width <= 12);
        assert!(protocol.protocol.size().height <= 6);

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn rejects_corrupt_images_without_panicking() {
        let path = temporary_path("jpg");
        fs::write(&path, b"not an image").unwrap();

        assert!(
            prepare(
                &Picker::halfblocks(),
                &PersistentCache::disabled(),
                &path,
                Size::new(12, 6),
                "7".parse().unwrap(),
                &KittySession::with_tmux(false),
            )
            .is_err()
        );

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn corrupt_thumbnail_reaches_a_visible_error_state() {
        let path = temporary_path("png");
        fs::write(&path, b"not an image").unwrap();
        let key = ThumbnailKey {
            entry_id: EntryId(1),
            revision: SourceRevision {
                size: 12,
                modified_nanoseconds: 1,
            },
            size: Size::new(12, 6),
            quality: "7".parse().unwrap(),
        };
        let mut thumbnails = Thumbnails::new(Picker::halfblocks());
        let generation = thumbnails.begin_generation();
        assert!(thumbnails.request(key, Arc::from(path.clone()), generation));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            thumbnails.drain();
            if let ThumbnailDisplay::Error(error) = thumbnails.display(key) {
                assert!(!error.is_empty());
                break;
            }
            assert!(Instant::now() < deadline, "thumbnail worker timed out");
            thread::sleep(Duration::from_millis(5));
        }
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn retained_thumbnail_failures_are_bounded() {
        let mut thumbnails = Thumbnails::new(Picker::halfblocks());
        let key = |entry_id| ThumbnailKey {
            entry_id: EntryId(entry_id),
            revision: SourceRevision::default(),
            size: Size::new(12, 6),
            quality: "7".parse().unwrap(),
        };
        for entry_id in 0..=MAX_CACHE_ENTRIES as u64 {
            thumbnails.insert_failure(key(entry_id), "decode failed".into());
        }

        assert_eq!(thumbnails.failed.len(), MAX_CACHE_ENTRIES);
        assert!(matches!(
            thumbnails.display(key(0)),
            ThumbnailDisplay::Loading
        ));
        assert!(matches!(
            thumbnails.display(key(MAX_CACHE_ENTRIES as u64)),
            ThumbnailDisplay::Error(_)
        ));
    }

    #[test]
    fn higher_quality_preserves_more_fine_detail_contrast() {
        let source = RgbImage::from_fn(512, 288, |x, y| {
            let grain = ((x.wrapping_mul(17) ^ y.wrapping_mul(29)) & 31) as u8;
            let edge = if (x / 24 + y / 24).is_multiple_of(2) {
                72
            } else {
                0
            };
            Rgb([
                (x * 255 / 512) as u8,
                (y * 255 / 288) as u8,
                48_u8.saturating_add(edge).saturating_add(grain),
            ])
        });
        let picker = Picker::halfblocks();
        let (triangle, _) = resize_for_protocol(
            &picker,
            source.clone().into(),
            Size::new(32, 9),
            "2".parse().unwrap(),
        )
        .unwrap();
        let (selected, _) = resize_for_protocol(
            &picker,
            source.into(),
            Size::new(32, 9),
            "9".parse().unwrap(),
        )
        .unwrap();
        let triangle = triangle.to_rgb8();
        let selected = selected.to_rgb8();
        let edge_energy = |image: &RgbImage| {
            (0..image.height())
                .flat_map(|y| {
                    (1..image.width()).map(move |x| {
                        image.get_pixel(x, y)[2].abs_diff(image.get_pixel(x - 1, y)[2]) as u64
                    })
                })
                .sum::<u64>()
        };

        let triangle_energy = edge_energy(&triangle);
        let selected_energy = edge_energy(&selected);
        assert!(
            selected_energy > triangle_energy,
            "Q9 edge energy {selected_energy} must exceed Q2 edge energy {triangle_energy}"
        );
    }

    #[test]
    fn halfblocks_quality_changes_final_rendered_cells() {
        let source = RgbImage::from_fn(640, 360, |x, y| {
            let checker = if (x / 7 + y / 7).is_multiple_of(2) {
                180
            } else {
                35
            };
            Rgb([checker, (x * 255 / 640) as u8, (y * 255 / 360) as u8])
        });
        let render = |quality: &str| {
            let picker = Picker::halfblocks();
            let protocol = prepare_image(
                &picker,
                source.clone().into(),
                Size::new(32, 12),
                quality.parse().unwrap(),
            )
            .unwrap();
            let PreparedThumbnail::Standard(protocol) = protocol else {
                panic!("Halfblocks must use the standard protocol wrapper");
            };
            let mut terminal = Terminal::new(TestBackend::new(32, 12)).unwrap();
            terminal
                .draw(|frame| frame.render_widget(Image::new(&protocol), frame.area()))
                .unwrap();
            terminal.backend().buffer().clone()
        };

        let q1 = render("1");
        let q9 = render("9");
        let changed_cells = q1
            .content()
            .iter()
            .zip(q9.content())
            .filter(|(low, high)| low != high)
            .count();
        assert!(
            changed_cells >= 20,
            "Q1 and Q9 changed only {changed_cells} final Halfblocks cells"
        );
    }

    #[test]
    fn persistent_source_requirement_matches_protocol_sample_density() {
        let halfblocks = Picker::halfblocks();
        assert_eq!(
            required_source_pixels(&halfblocks, Size::new(32, 12)),
            (32, 24)
        );

        let mut kitty = Picker::halfblocks();
        kitty.set_protocol_type(ProtocolType::Kitty);
        assert_eq!(
            required_source_pixels(&kitty, Size::new(32, 12)),
            (320, 240)
        );
    }

    #[test]
    fn halfblocks_fills_available_width_from_a_standard_cache_source() {
        let picker = Picker::halfblocks();
        let source = DynamicImage::ImageRgb8(RgbImage::from_pixel(128, 72, Rgb([20, 80, 160])));
        let protocol =
            prepare_image(&picker, source, Size::new(32, 12), "9".parse().unwrap()).unwrap();

        assert_eq!(protocol.size(), Size::new(32, 9));
    }

    #[test]
    fn cache_identity_separates_quality_levels() {
        let low = ThumbnailKey {
            entry_id: EntryId(42),
            revision: SourceRevision::default(),
            size: Size::new(32, 14),
            quality: "1".parse().unwrap(),
        };
        let high = ThumbnailKey {
            quality: "9".parse().unwrap(),
            ..low
        };

        assert_ne!(low, high);
        assert_eq!(HashSet::from([low, high]).len(), 2);
    }

    #[test]
    fn previous_quality_remains_available_while_replacement_is_pending() {
        let mut thumbnails = Thumbnails::new(Picker::halfblocks());
        let low = ThumbnailKey {
            entry_id: EntryId(7),
            revision: SourceRevision::default(),
            size: Size::new(32, 12),
            quality: "1".parse().unwrap(),
        };
        let high = ThumbnailKey {
            quality: "9".parse().unwrap(),
            ..low
        };
        let protocol = Halfblocks::new(
            DynamicImage::ImageRgb8(RgbImage::from_pixel(32, 24, Rgb([20, 80, 160]))),
            low.size,
        )
        .map(Protocol::Halfblocks)
        .map(PreparedThumbnail::Standard)
        .unwrap();
        thumbnails.insert(low, protocol);
        thumbnails.pending.insert(high);

        let low_pointer = thumbnails.cache.get(&low).unwrap() as *const PreparedThumbnail;
        let display_pointer = thumbnails.get_for_display(high).unwrap() as *const PreparedThumbnail;
        assert_eq!(display_pointer, low_pointer);
        assert!(
            thumbnails
                .get_for_display(ThumbnailKey {
                    entry_id: EntryId(8),
                    ..high
                })
                .is_none()
        );
        assert!(
            thumbnails
                .get_for_display(ThumbnailKey {
                    size: Size::new(31, 12),
                    ..high
                })
                .is_none()
        );
        assert!(
            thumbnails
                .get_for_display(ThumbnailKey {
                    revision: SourceRevision {
                        size: 1,
                        modified_nanoseconds: 2,
                    },
                    ..high
                })
                .is_none()
        );

        thumbnails.pending.remove(&high);
        assert!(matches!(
            thumbnails.display(high),
            ThumbnailDisplay::Loading
        ));
        thumbnails.insert_failure(high, "quality preparation failed".into());
        assert!(matches!(
            thumbnails.display(high),
            ThumbnailDisplay::Error("quality preparation failed")
        ));
    }

    #[test]
    fn failures_retry_after_backoff_and_become_permanent_after_three_attempts() {
        let (request_sender, _request_receiver) = mpsc::sync_channel(8);
        let (_response_sender, response_receiver) = mpsc::sync_channel(8);
        let mut thumbnails = Thumbnails {
            request_sender,
            response_receiver,
            cache: HashMap::new(),
            insertion_order: VecDeque::new(),
            cache_bytes: 0,
            cache_entry_limit: MAX_CACHE_ENTRIES,
            cache_byte_limit: MAX_CACHE_BYTES,
            visible: HashSet::new(),
            pending: HashSet::new(),
            deferred: HashSet::new(),
            failed: HashMap::new(),
            failure_order: VecDeque::new(),
            generation: Arc::new(AtomicU64::new(0)),
            persistent_hits: 0,
            persistent_misses: 0,
            response_closed: false,
        };
        let key = ThumbnailKey {
            entry_id: EntryId(3),
            revision: SourceRevision::default(),
            size: Size::new(4, 2),
            quality: "7".parse().unwrap(),
        };
        let start = Instant::now();
        let request = |thumbnails: &mut Thumbnails, now| {
            thumbnails.request_at(key, Arc::from(Path::new("broken.png")), 1, now)
        };

        thumbnails.insert_failure_at(key, "first".into(), start);
        assert!(!request(
            &mut thumbnails,
            start + Duration::from_millis(1999)
        ));
        assert!(matches!(
            thumbnails.display(key),
            ThumbnailDisplay::Error("first")
        ));
        assert!(request(&mut thumbnails, start + Duration::from_secs(2)));
        assert!(matches!(
            thumbnails.display(key),
            ThumbnailDisplay::Error("first")
        ));
        assert!(!request(&mut thumbnails, start + Duration::from_secs(3)));

        thumbnails.pending.remove(&key);
        thumbnails.insert_failure_at(key, "second".into(), start + Duration::from_secs(3));
        assert_eq!(thumbnails.failure_attempts(key), Some(2));
        assert!(!request(&mut thumbnails, start + Duration::from_secs(10)));
        assert!(request(&mut thumbnails, start + Duration::from_secs(11)));

        thumbnails.pending.remove(&key);
        thumbnails.insert_failure_at(key, "third".into(), start + Duration::from_secs(11));
        assert_eq!(thumbnails.failure_attempts(key), Some(3));
        assert!(!request(
            &mut thumbnails,
            start + Duration::from_secs(10_000)
        ));
        assert!(matches!(
            thumbnails.display(key),
            ThumbnailDisplay::Error("third")
        ));

        let prepared = Halfblocks::new(
            DynamicImage::ImageRgb8(RgbImage::from_pixel(4, 4, Rgb([20, 80, 160]))),
            Size::new(4, 2),
        )
        .map(Protocol::Halfblocks)
        .map(PreparedThumbnail::Standard)
        .unwrap();
        thumbnails.insert(key, prepared);
        assert_eq!(thumbnails.failure_attempts(key), None);
        assert!(matches!(
            thumbnails.display(key),
            ThumbnailDisplay::Ready(_)
        ));
    }

    #[test]
    fn deferred_requests_keep_the_older_quality_until_a_new_generation() {
        let (request_sender, _request_receiver) = mpsc::sync_channel(1);
        let (_response_sender, response_receiver) = mpsc::sync_channel(1);
        let mut thumbnails = Thumbnails {
            request_sender,
            response_receiver,
            cache: HashMap::new(),
            insertion_order: VecDeque::new(),
            cache_bytes: 0,
            cache_entry_limit: MAX_CACHE_ENTRIES,
            cache_byte_limit: MAX_CACHE_BYTES,
            visible: HashSet::new(),
            pending: HashSet::new(),
            deferred: HashSet::new(),
            failed: HashMap::new(),
            failure_order: VecDeque::new(),
            generation: Arc::new(AtomicU64::new(0)),
            persistent_hits: 0,
            persistent_misses: 0,
            response_closed: false,
        };
        let low = |entry_id| ThumbnailKey {
            entry_id: EntryId(entry_id),
            revision: SourceRevision::default(),
            size: Size::new(4, 2),
            quality: "1".parse().unwrap(),
        };
        let high = |entry_id| ThumbnailKey {
            quality: "9".parse().unwrap(),
            ..low(entry_id)
        };
        let prepared = || {
            Halfblocks::new(
                DynamicImage::ImageRgb8(RgbImage::from_pixel(4, 4, Rgb([20, 80, 160]))),
                Size::new(4, 2),
            )
            .map(Protocol::Halfblocks)
            .map(PreparedThumbnail::Standard)
            .unwrap()
        };
        thumbnails.insert(low(1), prepared());
        thumbnails.insert(low(2), prepared());
        let generation = thumbnails.begin_generation();

        assert!(thumbnails.request(high(1), Arc::from(Path::new("one.png")), generation));
        assert!(!thumbnails.request(high(2), Arc::from(Path::new("two.png")), generation));
        assert!(thumbnails.pending.contains(&high(1)));
        assert!(thumbnails.deferred.contains(&high(2)));
        assert!(matches!(
            thumbnails.display(high(2)),
            ThumbnailDisplay::Ready(_)
        ));

        let generation = thumbnails.begin_generation();
        assert!(thumbnails.deferred.is_empty());
        assert!(matches!(
            thumbnails.display(high(2)),
            ThumbnailDisplay::Loading
        ));
        assert!(!thumbnails.request(high(2), Arc::from(Path::new("two.png")), generation));
        assert!(thumbnails.deferred.contains(&high(2)));
        thumbnails.insert_failure(high(2), "decode failed".into());
        assert!(matches!(
            thumbnails.display(high(2)),
            ThumbnailDisplay::Error("decode failed")
        ));
    }

    #[test]
    fn prepared_cache_evicts_by_bytes_and_rejects_oversized_entries() {
        let key = |entry_id| ThumbnailKey {
            entry_id: EntryId(entry_id),
            revision: SourceRevision::default(),
            size: Size::new(4, 2),
            quality: "7".parse().unwrap(),
        };
        let prepared = || {
            Halfblocks::new(
                DynamicImage::ImageRgb8(RgbImage::from_pixel(4, 4, Rgb([20, 80, 160]))),
                Size::new(4, 2),
            )
            .map(Protocol::Halfblocks)
            .map(PreparedThumbnail::Standard)
            .unwrap()
        };
        let one_entry_bytes = prepared().resident_bytes();
        let mut thumbnails = Thumbnails::with_limits(
            Picker::halfblocks(),
            KittySession::with_tmux(false),
            10,
            one_entry_bytes,
            MAX_PENDING_REQUESTS,
        );
        thumbnails.insert(key(1), prepared());
        thumbnails.insert(key(2), prepared());
        assert_eq!(thumbnails.cache_stats(), (1, one_entry_bytes));
        assert!(!thumbnails.cache.contains_key(&key(1)));
        assert!(thumbnails.cache.contains_key(&key(2)));

        let mut too_small = Thumbnails::with_limits(
            Picker::halfblocks(),
            KittySession::with_tmux(false),
            10,
            one_entry_bytes - 1,
            MAX_PENDING_REQUESTS,
        );
        too_small.insert(key(3), prepared());
        assert_eq!(too_small.cache_stats(), (0, 0));
        assert!(matches!(
            too_small.display(key(3)),
            ThumbnailDisplay::Error(error) if error.contains("memory limit")
        ));
        assert!(!too_small.request(key(3), Arc::from(Path::new("unused.png")), 0));

        let mut lru = Thumbnails::with_limits(
            Picker::halfblocks(),
            KittySession::with_tmux(false),
            2,
            one_entry_bytes * 3,
            MAX_PENDING_REQUESTS,
        );
        lru.insert(key(1), prepared());
        lru.insert(key(2), prepared());
        assert!(matches!(lru.display(key(1)), ThumbnailDisplay::Ready(_)));
        lru.insert(key(3), prepared());
        assert!(lru.cache.contains_key(&key(1)));
        assert!(!lru.cache.contains_key(&key(2)));
        assert!(lru.cache.contains_key(&key(3)));
    }

    #[test]
    fn raw_thumbnail_limit_is_checked_before_resize_allocation() {
        let picker = Picker::halfblocks();
        let error = resize_for_protocol(
            &picker,
            DynamicImage::new_rgb8(1, 1),
            Size::new(u16::MAX, u16::MAX),
            "7".parse().unwrap(),
        )
        .unwrap_err();
        assert!(error.contains("64 MiB"));
    }

    #[test]
    fn kitty_thumbnail_rearms_only_after_leaving_exact_visibility() {
        use crate::kitty::KittyImage;

        let session = KittySession::with_tmux(false);
        let key = ThumbnailKey {
            entry_id: EntryId(1),
            revision: SourceRevision::default(),
            size: Size::new(1, 1),
            quality: "7".parse().unwrap(),
        };
        let prepared = PreparedThumbnail::Kitty(
            KittyProtocol::new(
                DynamicImage::new_rgba8(1, 1),
                Size::new(1, 1),
                session.clone(),
            )
            .unwrap(),
        );
        let mut thumbnails = Thumbnails::with_session(Picker::halfblocks(), session);
        thumbnails.insert(key, prepared);
        assert!(!thumbnails.update_visible([key]));

        let PreparedThumbnail::Kitty(protocol) = thumbnails.cache.get(&key).unwrap() else {
            panic!("test inserted a Kitty protocol");
        };
        let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(KittyImage::new(protocol), frame.area()))
            .unwrap();
        assert!(!thumbnails.update_visible([key]));
        assert!(!thumbnails.update_visible([]));
        assert!(thumbnails.update_visible([key]));
    }
}
