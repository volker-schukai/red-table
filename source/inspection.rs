use std::{
    collections::{HashMap, VecDeque},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
};

use image::{DynamicImage, Rgba, RgbaImage, imageops::FilterType};
use ratatui::layout::Size;
use ratatui_image::{
    FontSize,
    picker::{Picker, ProtocolType},
};

use crate::{
    app::{EntryId, SourceRevision},
    image_source::{decode_oriented, decoded_bytes},
    kitty::KittySession,
    settings::{BackgroundMode, ThumbnailQuality},
    thumbnails::{PreparedThumbnail, encode_exact},
};

const MAX_PENDING_REQUESTS: usize = 4;
const MAX_PREPARED_VIEW_BYTES: u64 = 128 * 1024 * 1024;
const PAN_STEP: u16 = 500;
const CENTER: u16 = 5_000;
const ZOOM_LEVELS: [u16; 6] = [25, 50, 100, 200, 400, 800];

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum Zoom {
    Fit,
    Percent(u16),
}

impl Zoom {
    pub(crate) const fn label(self) -> &'static str {
        match self {
            Self::Fit => "fit",
            Self::Percent(25) => "25%",
            Self::Percent(50) => "50%",
            Self::Percent(100) => "100%",
            Self::Percent(200) => "200%",
            Self::Percent(400) => "400%",
            Self::Percent(800) => "800%",
            Self::Percent(_) => "custom",
        }
    }

    pub(crate) const fn toggle_fit_hundred(self) -> Self {
        match self {
            Self::Fit => Self::Percent(100),
            Self::Percent(_) => Self::Fit,
        }
    }

    pub(crate) fn zoom_in(self) -> Self {
        match self {
            Self::Fit => Self::Percent(100),
            Self::Percent(percent) => Self::Percent(
                ZOOM_LEVELS
                    .iter()
                    .copied()
                    .find(|level| *level > percent)
                    .unwrap_or(800),
            ),
        }
    }

    pub(crate) fn zoom_out(self) -> Self {
        match self {
            Self::Fit => Self::Fit,
            Self::Percent(percent) => ZOOM_LEVELS
                .iter()
                .copied()
                .rev()
                .find(|level| *level < percent)
                .map(Self::Percent)
                .unwrap_or(Self::Fit),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ViewState {
    pub(crate) zoom: Zoom,
    pub(crate) center_x: u16,
    pub(crate) center_y: u16,
}

impl Default for ViewState {
    fn default() -> Self {
        Self {
            zoom: Zoom::Fit,
            center_x: CENTER,
            center_y: CENTER,
        }
    }
}

impl ViewState {
    pub(crate) fn recenter(&mut self) {
        self.center_x = CENTER;
        self.center_y = CENTER;
    }

    pub(crate) fn pan_left(&mut self) {
        self.center_x = self.center_x.saturating_sub(PAN_STEP);
    }

    pub(crate) fn pan_right(&mut self) {
        self.center_x = self.center_x.saturating_add(PAN_STEP).min(10_000);
    }

    pub(crate) fn pan_up(&mut self) {
        self.center_y = self.center_y.saturating_sub(PAN_STEP);
    }

    pub(crate) fn pan_down(&mut self) {
        self.center_y = self.center_y.saturating_add(PAN_STEP).min(10_000);
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct InspectionKey {
    pub(crate) entry_id: EntryId,
    pub(crate) revision: SourceRevision,
    pub(crate) size: Size,
    pub(crate) zoom: Zoom,
    pub(crate) center_x: u16,
    pub(crate) center_y: u16,
    pub(crate) background: BackgroundMode,
    pub(crate) quality: ThumbnailQuality,
}

struct Request {
    key: InspectionKey,
    path: Arc<Path>,
    generation: u64,
}

struct Response {
    key: InspectionKey,
    generation: u64,
    prepared: Option<Result<PreparedThumbnail, String>>,
}

pub(crate) enum InspectionDisplay<'a> {
    Loading,
    Ready(&'a PreparedThumbnail),
    Error(&'a str),
}

pub(crate) struct Inspection {
    request_sender: mpsc::SyncSender<Request>,
    response_receiver: mpsc::Receiver<Response>,
    generation: Arc<AtomicU64>,
    schedule_gate: Arc<Mutex<()>>,
    desired: Option<InspectionKey>,
    ready: Option<ReadyFrame>,
    error: Option<(InspectionKey, String)>,
    visible_frame: Option<u64>,
    next_frame_id: u64,
    response_closed: bool,
}

struct ReadyFrame {
    key: InspectionKey,
    id: u64,
    prepared: PreparedThumbnail,
}

#[derive(Clone)]
pub(crate) struct DecodedSources {
    cache: Arc<Mutex<DecodedCache>>,
}

impl DecodedSources {
    pub(crate) fn new(limit: u64) -> Self {
        Self {
            cache: Arc::new(Mutex::new(DecodedCache::new(limit))),
        }
    }
}

impl Inspection {
    #[cfg(test)]
    pub(crate) fn new(picker: Picker, decoded_sources: DecodedSources) -> Self {
        Self::with_session(picker, decoded_sources, KittySession::from_process())
    }

    pub(crate) fn with_session(
        picker: Picker,
        decoded_sources: DecodedSources,
        kitty_session: KittySession,
    ) -> Self {
        let (request_sender, request_receiver) = mpsc::sync_channel(MAX_PENDING_REQUESTS);
        let (response_sender, response_receiver) = mpsc::sync_channel(1);
        let generation = Arc::new(AtomicU64::new(0));
        let worker_generation = Arc::clone(&generation);
        let schedule_gate = Arc::new(Mutex::new(()));
        let worker_schedule_gate = Arc::clone(&schedule_gate);
        thread::Builder::new()
            .name("red-table-inspection".into())
            .spawn(move || {
                worker(
                    request_receiver,
                    response_sender,
                    picker,
                    kitty_session,
                    worker_generation,
                    worker_schedule_gate,
                    decoded_sources,
                );
            })
            .expect("inspection worker should start");
        Self {
            request_sender,
            response_receiver,
            generation,
            schedule_gate,
            desired: None,
            ready: None,
            error: None,
            visible_frame: None,
            next_frame_id: 0,
            response_closed: false,
        }
    }

    pub(crate) fn request(&mut self, key: InspectionKey, path: Arc<Path>) -> bool {
        if key.size.width == 0 || key.size.height == 0 || self.desired == Some(key) {
            return false;
        }
        if self.response_closed {
            self.desired = Some(key);
            self.ready = None;
            self.error = Some((key, "image inspection worker is unavailable".into()));
            return true;
        }
        let _gate = self
            .schedule_gate
            .lock()
            .expect("inspection scheduling mutex should not be poisoned");
        let generation = self.generation.load(Ordering::Acquire) + 1;
        match self.request_sender.try_send(Request {
            key,
            path,
            generation,
        }) {
            Ok(()) => {
                self.generation.store(generation, Ordering::Release);
                self.desired = Some(key);
                true
            }
            Err(mpsc::TrySendError::Full(_)) => false,
            Err(mpsc::TrySendError::Disconnected(_)) => {
                self.response_closed = true;
                self.desired = Some(key);
                self.ready = None;
                self.error = Some((key, "image inspection worker is unavailable".into()));
                true
            }
        }
    }

    pub(crate) fn drain(&mut self) -> bool {
        let mut changed = false;
        loop {
            match self.response_receiver.try_recv() {
                Ok(response) => {
                    changed = true;
                    if response.generation != self.generation.load(Ordering::Acquire)
                        || self.desired != Some(response.key)
                    {
                        continue;
                    }
                    match response.prepared {
                        Some(Ok(prepared)) => {
                            self.store_ready(response.key, prepared);
                            self.error = None;
                        }
                        Some(Err(error)) => {
                            self.error = Some((response.key, error));
                            self.ready = None;
                        }
                        None => {}
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !self.response_closed {
                        self.response_closed = true;
                        if let Some(key) = self.desired {
                            self.ready = None;
                            self.error =
                                Some((key, "image inspection worker stopped unexpectedly".into()));
                            changed = true;
                        }
                    }
                    break;
                }
            }
        }
        changed
    }

    fn store_ready(&mut self, key: InspectionKey, prepared: PreparedThumbnail) {
        let id = self.next_frame_id;
        self.next_frame_id += 1;
        self.ready = Some(ReadyFrame { key, id, prepared });
    }

    /// Resolves the frame shown for `key`: the exact result, or, while the
    /// exact key is still pending, the last ready frame of the same source
    /// An exact error always wins over the fallback.
    fn displayed_frame(&self, key: InspectionKey) -> Option<&ReadyFrame> {
        let frame = self.ready.as_ref()?;
        if frame.key == key {
            return Some(frame);
        }
        if self
            .error
            .as_ref()
            .is_some_and(|(error_key, _)| *error_key == key)
        {
            return None;
        }
        let pending_same_source = self.desired == Some(key)
            && frame.key.entry_id == key.entry_id
            && frame.key.revision == key.revision;
        pending_same_source.then_some(frame)
    }

    pub(crate) fn display(&self, key: InspectionKey) -> InspectionDisplay<'_> {
        if let Some(frame) = self.displayed_frame(key) {
            return InspectionDisplay::Ready(&frame.prepared);
        }
        if let Some((error_key, error)) = self.error.as_ref()
            && *error_key == key
        {
            return InspectionDisplay::Error(error);
        }
        InspectionDisplay::Loading
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.desired.is_some_and(|key| {
            self.ready.as_ref().is_none_or(|frame| frame.key != key)
                && self
                    .error
                    .as_ref()
                    .is_none_or(|(error_key, _)| *error_key != key)
        })
    }

    pub(crate) fn update_visible(&mut self, key: Option<InspectionKey>) -> bool {
        let displayed = key.and_then(|key| self.displayed_frame(key));
        let displayed_id = displayed.map(|frame| frame.id);
        let rearmed = displayed_id != self.visible_frame
            && displayed.is_some_and(|frame| frame.prepared.rearm());
        self.visible_frame = displayed_id;
        rearmed
    }
}

fn worker(
    receiver: mpsc::Receiver<Request>,
    sender: mpsc::SyncSender<Response>,
    picker: Picker,
    kitty_session: KittySession,
    generation: Arc<AtomicU64>,
    schedule_gate: Arc<Mutex<()>>,
    decoded_sources: DecodedSources,
) {
    while let Ok(request) = receiver.recv() {
        let current = {
            let _gate = schedule_gate
                .lock()
                .expect("inspection scheduling mutex should not be poisoned");
            request.generation == generation.load(Ordering::Acquire)
        };
        let prepared = if !current {
            None
        } else {
            let cached = {
                decoded_sources
                    .cache
                    .lock()
                    .expect("decoded cache mutex should not be poisoned")
                    .get(request.key.entry_id, request.key.revision)
            };
            let source = cached.map(Ok).unwrap_or_else(|| {
                decode_oriented(&request.path).map(|image| {
                    let image = Arc::new(image);
                    decoded_sources
                        .cache
                        .lock()
                        .expect("decoded cache mutex should not be poisoned")
                        .insert(
                            request.key.entry_id,
                            request.key.revision,
                            Arc::clone(&image),
                        );
                    image
                })
            });
            if request.generation != generation.load(Ordering::Acquire) {
                None
            } else {
                Some(source.and_then(|source| {
                    prepare_view_with_session(&picker, &source, request.key, &kitty_session)
                }))
            }
        };
        let prepared = (request.generation == generation.load(Ordering::Acquire))
            .then_some(prepared)
            .flatten();
        if sender
            .send(Response {
                key: request.key,
                generation: request.generation,
                prepared,
            })
            .is_err()
        {
            return;
        }
    }
}

struct CachedSource {
    revision: SourceRevision,
    image: Arc<DynamicImage>,
    bytes: u64,
}

struct DecodedCache {
    entries: HashMap<EntryId, CachedSource>,
    order: VecDeque<EntryId>,
    bytes: u64,
    limit: u64,
}

impl DecodedCache {
    fn new(limit: u64) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            bytes: 0,
            limit,
        }
    }

    fn get(&mut self, id: EntryId, revision: SourceRevision) -> Option<Arc<DynamicImage>> {
        let image = self
            .entries
            .get(&id)
            .filter(|cached| cached.revision == revision)
            .map(|cached| Arc::clone(&cached.image))?;
        self.order.retain(|candidate| *candidate != id);
        self.order.push_back(id);
        Some(image)
    }

    fn insert(&mut self, id: EntryId, revision: SourceRevision, image: Arc<DynamicImage>) {
        let bytes = decoded_bytes(&image);
        if bytes > self.limit {
            return;
        }
        if let Some(previous) = self.entries.remove(&id) {
            self.bytes = self.bytes.saturating_sub(previous.bytes);
            self.order.retain(|candidate| *candidate != id);
        }
        while self.bytes.saturating_add(bytes) > self.limit {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some(removed) = self.entries.remove(&oldest) {
                self.bytes = self.bytes.saturating_sub(removed.bytes);
            }
        }
        self.entries.insert(
            id,
            CachedSource {
                revision,
                image,
                bytes,
            },
        );
        self.order.push_back(id);
        self.bytes += bytes;
    }
}

#[cfg(test)]
fn prepare_view(
    picker: &Picker,
    source: &DynamicImage,
    key: InspectionKey,
) -> Result<PreparedThumbnail, String> {
    prepare_view_with_session(picker, source, key, &KittySession::from_process())
}

fn prepare_view_with_session(
    picker: &Picker,
    source: &DynamicImage,
    key: InspectionKey,
    kitty_session: &KittySession,
) -> Result<PreparedThumbnail, String> {
    let font = sample_size(picker);
    let target_width = u32::from(key.size.width) * u32::from(font.width);
    let target_height = u32::from(key.size.height) * u32::from(font.height);
    let prepared_bytes = u64::from(target_width)
        .saturating_mul(u64::from(target_height))
        .saturating_mul(4);
    if prepared_bytes > MAX_PREPARED_VIEW_BYTES {
        return Err("terminal view exceeds the 128 MiB prepared-image limit".into());
    }
    let mut canvas = background(target_width, target_height, key.background);
    let rendered = match key.zoom {
        Zoom::Fit => crate::image_source::downscale(
            source,
            target_width,
            target_height,
            FilterType::Lanczos3,
        ),
        Zoom::Percent(percent) => {
            let geometry = crop_geometry(
                source.width(),
                source.height(),
                target_width,
                target_height,
                percent,
                key.center_x,
                key.center_y,
            );
            source
                .crop_imm(geometry.x, geometry.y, geometry.width, geometry.height)
                .resize_exact(
                    geometry.display_width,
                    geometry.display_height,
                    FilterType::Lanczos3,
                )
        }
    };
    let x = i64::from(target_width.saturating_sub(rendered.width()) / 2);
    let y = i64::from(target_height.saturating_sub(rendered.height()) / 2);
    image::imageops::overlay(&mut canvas, &rendered.into_rgba8(), x, y);
    encode_exact(
        picker,
        DynamicImage::ImageRgba8(canvas),
        key.size,
        kitty_session,
    )
}

fn sample_size(picker: &Picker) -> FontSize {
    if picker.protocol_type() == ProtocolType::Halfblocks {
        FontSize::new(1, 2)
    } else {
        picker.font_size()
    }
}

fn background(width: u32, height: u32, mode: BackgroundMode) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        let value = match mode {
            BackgroundMode::Checkerboard if (x / 8 + y / 8).is_multiple_of(2) => 72,
            BackgroundMode::Checkerboard => 112,
            BackgroundMode::Dark => 24,
            BackgroundMode::Light => 224,
        };
        Rgba([value, value, value, 255])
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CropGeometry {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    display_width: u32,
    display_height: u32,
}

fn crop_geometry(
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
    percent: u16,
    center_x: u16,
    center_y: u16,
) -> CropGeometry {
    let percent = u32::from(percent.max(1));
    let width = target_width
        .saturating_mul(100)
        .div_ceil(percent)
        .clamp(1, source_width.max(1));
    let height = target_height
        .saturating_mul(100)
        .div_ceil(percent)
        .clamp(1, source_height.max(1));
    let x = u64::from(source_width.saturating_sub(width))
        .saturating_mul(u64::from(center_x.min(10_000)))
        / 10_000;
    let y = u64::from(source_height.saturating_sub(height))
        .saturating_mul(u64::from(center_y.min(10_000)))
        / 10_000;
    CropGeometry {
        x: x as u32,
        y: y as u32,
        width,
        height,
        display_width: width
            .saturating_mul(percent)
            .div_ceil(100)
            .min(target_width),
        display_height: height
            .saturating_mul(percent)
            .div_ceil(100)
            .min(target_height),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs, thread,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use image::RgbImage;
    use ratatui::{Terminal, backend::TestBackend};

    use crate::kitty::{KittyImage, KittyProtocol};

    use super::*;

    #[test]
    fn zoom_and_pan_are_bounded() {
        let mut state = ViewState::default();
        assert_eq!(state.zoom.toggle_fit_hundred(), Zoom::Percent(100));
        assert_eq!(Zoom::Percent(100).zoom_in(), Zoom::Percent(200));
        assert_eq!(Zoom::Percent(25).zoom_out(), Zoom::Fit);
        for _ in 0..100 {
            state.pan_left();
            state.pan_up();
        }
        assert_eq!((state.center_x, state.center_y), (0, 0));
        for _ in 0..100 {
            state.pan_right();
            state.pan_down();
        }
        assert_eq!((state.center_x, state.center_y), (10_000, 10_000));
    }

    #[test]
    fn hundred_percent_crop_maps_one_source_pixel_to_one_sample() {
        assert_eq!(
            crop_geometry(100, 80, 20, 10, 100, 5_000, 5_000),
            CropGeometry {
                x: 40,
                y: 35,
                width: 20,
                height: 10,
                display_width: 20,
                display_height: 10,
            }
        );
        let edge = crop_geometry(100, 80, 20, 10, 200, 10_000, 10_000);
        assert_eq!((edge.x + edge.width, edge.y + edge.height), (100, 80));
    }

    #[test]
    fn decoded_cache_evicts_by_bytes_and_revision() {
        let mut cache = DecodedCache::new(32);
        let first = Arc::new(DynamicImage::ImageRgb8(RgbImage::new(2, 2)));
        let second = Arc::new(DynamicImage::ImageRgb8(RgbImage::new(3, 3)));
        cache.insert(EntryId(1), SourceRevision::default(), first);
        cache.insert(EntryId(2), SourceRevision::default(), second);
        assert!(cache.get(EntryId(1), SourceRevision::default()).is_none());
        assert!(cache.get(EntryId(2), SourceRevision::default()).is_some());
        assert!(
            cache
                .get(
                    EntryId(2),
                    SourceRevision {
                        size: 1,
                        modified_nanoseconds: 1,
                    }
                )
                .is_none()
        );
    }

    #[test]
    fn decoded_sources_share_one_cache_and_byte_limit() {
        let sources = DecodedSources::new(32);
        let clone = sources.clone();
        assert!(Arc::ptr_eq(&sources.cache, &clone.cache));
        let first = Arc::new(DynamicImage::ImageRgb8(RgbImage::new(2, 2)));
        let second = Arc::new(DynamicImage::ImageRgb8(RgbImage::new(3, 3)));
        sources
            .cache
            .lock()
            .unwrap()
            .insert(EntryId(1), SourceRevision::default(), first);
        clone
            .cache
            .lock()
            .unwrap()
            .insert(EntryId(2), SourceRevision::default(), second);
        let mut cache = sources.cache.lock().unwrap();
        assert!(cache.get(EntryId(1), SourceRevision::default()).is_none());
        assert!(cache.get(EntryId(2), SourceRevision::default()).is_some());
        assert!(cache.bytes <= cache.limit);
    }

    #[test]
    fn transparent_source_is_composited_on_selected_background() {
        let source = DynamicImage::new_rgba8(2, 2);
        let key = InspectionKey {
            entry_id: EntryId(1),
            revision: SourceRevision::default(),
            size: Size::new(2, 1),
            zoom: Zoom::Fit,
            center_x: CENTER,
            center_y: CENTER,
            background: BackgroundMode::Light,
            quality: "7".parse().unwrap(),
        };
        let prepared = prepare_view(&Picker::halfblocks(), &source, key).unwrap();
        assert_eq!(prepared.size(), Size::new(2, 1));
    }

    #[test]
    fn oversized_terminal_view_is_rejected_before_allocation() {
        let source = DynamicImage::new_rgb8(2, 2);
        let key = InspectionKey {
            entry_id: EntryId(1),
            revision: SourceRevision::default(),
            size: Size::new(u16::MAX, u16::MAX),
            zoom: Zoom::Fit,
            center_x: CENTER,
            center_y: CENTER,
            background: BackgroundMode::Dark,
            quality: "7".parse().unwrap(),
        };
        let error = match prepare_view(&Picker::halfblocks(), &source, key) {
            Ok(_) => panic!("oversized view must fail"),
            Err(error) => error,
        };
        assert!(error.contains("128 MiB"));
    }

    #[test]
    fn retained_kitty_inspection_rearms_after_becoming_hidden() {
        let key = InspectionKey {
            entry_id: EntryId(1),
            revision: SourceRevision::default(),
            size: Size::new(1, 1),
            zoom: Zoom::Fit,
            center_x: CENTER,
            center_y: CENTER,
            background: BackgroundMode::Dark,
            quality: "7".parse().unwrap(),
        };
        let session = KittySession::with_tmux(false);
        let prepared = PreparedThumbnail::Kitty(
            KittyProtocol::new(DynamicImage::new_rgba8(1, 1), Size::new(1, 1), session).unwrap(),
        );
        let mut inspection =
            Inspection::new(Picker::halfblocks(), DecodedSources::new(64 * 1024 * 1024));
        inspection.store_ready(key, prepared);
        assert!(!inspection.update_visible(Some(key)));

        let PreparedThumbnail::Kitty(protocol) = &inspection.ready.as_ref().unwrap().prepared
        else {
            panic!("test inserted a Kitty protocol");
        };
        let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(KittyImage::new(protocol), frame.area()))
            .unwrap();
        assert!(!inspection.update_visible(Some(key)));
        assert!(!inspection.update_visible(None));
        assert!(inspection.update_visible(Some(key)));
    }

    fn detached_inspection() -> (
        Inspection,
        mpsc::Receiver<Request>,
        mpsc::SyncSender<Response>,
    ) {
        let (request_sender, request_receiver) = mpsc::sync_channel(16);
        let (response_sender, response_receiver) = mpsc::sync_channel(16);
        let inspection = Inspection {
            request_sender,
            response_receiver,
            generation: Arc::new(AtomicU64::new(0)),
            schedule_gate: Arc::new(Mutex::new(())),
            desired: None,
            ready: None,
            error: None,
            visible_frame: None,
            next_frame_id: 0,
            response_closed: false,
        };
        (inspection, request_receiver, response_sender)
    }

    fn halfblock_frame() -> PreparedThumbnail {
        prepare_view(
            &Picker::halfblocks(),
            &DynamicImage::new_rgb8(2, 2),
            InspectionKey {
                entry_id: EntryId(1),
                revision: SourceRevision::default(),
                size: Size::new(2, 1),
                zoom: Zoom::Fit,
                center_x: CENTER,
                center_y: CENTER,
                background: BackgroundMode::Dark,
                quality: "7".parse().unwrap(),
            },
        )
        .unwrap()
    }

    #[test]
    fn pending_same_source_keys_keep_showing_the_last_ready_frame() {
        let (mut inspection, _requests, _responses) = detached_inspection();
        let fit = InspectionKey {
            entry_id: EntryId(1),
            revision: SourceRevision::default(),
            size: Size::new(8, 4),
            zoom: Zoom::Fit,
            center_x: CENTER,
            center_y: CENTER,
            background: BackgroundMode::Dark,
            quality: "7".parse().unwrap(),
        };
        inspection.desired = Some(fit);
        inspection.store_ready(fit, halfblock_frame());
        let shown = |inspection: &Inspection, key| match inspection.display(key) {
            InspectionDisplay::Ready(prepared) => Some(prepared as *const PreparedThumbnail),
            _ => None,
        };
        let exact = shown(&inspection, fit).unwrap();

        let panned = InspectionKey {
            zoom: Zoom::Percent(200),
            center_x: CENTER + PAN_STEP,
            ..fit
        };
        assert!(inspection.request(panned, Arc::from(Path::new("image.png"))));
        assert!(inspection.is_pending());
        assert_eq!(shown(&inspection, panned), Some(exact));
        let recolored = InspectionKey {
            background: BackgroundMode::Light,
            ..panned
        };
        assert!(inspection.request(recolored, Arc::from(Path::new("image.png"))));
        assert_eq!(shown(&inspection, recolored), Some(exact));

        let other_image = InspectionKey {
            entry_id: EntryId(2),
            ..fit
        };
        assert!(inspection.request(other_image, Arc::from(Path::new("other.png"))));
        assert!(matches!(
            inspection.display(other_image),
            InspectionDisplay::Loading
        ));

        let changed_revision = InspectionKey {
            revision: SourceRevision {
                size: 1,
                modified_nanoseconds: 1,
            },
            ..fit
        };
        assert!(inspection.request(changed_revision, Arc::from(Path::new("image.png"))));
        assert!(matches!(
            inspection.display(changed_revision),
            InspectionDisplay::Loading
        ));

        assert!(inspection.request(panned, Arc::from(Path::new("image.png"))));
        inspection.error = Some((panned, "decode failed".into()));
        assert!(matches!(
            inspection.display(panned),
            InspectionDisplay::Error("decode failed")
        ));
        assert!(!inspection.is_pending());
    }

    #[test]
    fn fallback_frame_is_not_rearmed_while_it_stays_on_screen() {
        let (mut inspection, _requests, _responses) = detached_inspection();
        let fit = InspectionKey {
            entry_id: EntryId(1),
            revision: SourceRevision::default(),
            size: Size::new(1, 1),
            zoom: Zoom::Fit,
            center_x: CENTER,
            center_y: CENTER,
            background: BackgroundMode::Dark,
            quality: "7".parse().unwrap(),
        };
        let session = KittySession::with_tmux(false);
        let prepared = PreparedThumbnail::Kitty(
            KittyProtocol::new(DynamicImage::new_rgba8(1, 1), Size::new(1, 1), session).unwrap(),
        );
        inspection.desired = Some(fit);
        inspection.store_ready(fit, prepared);
        assert!(!inspection.update_visible(Some(fit)));
        let PreparedThumbnail::Kitty(protocol) = &inspection.ready.as_ref().unwrap().prepared
        else {
            panic!("test inserted a Kitty protocol");
        };
        let mut terminal = Terminal::new(TestBackend::new(1, 1)).unwrap();
        terminal
            .draw(|frame| frame.render_widget(KittyImage::new(protocol), frame.area()))
            .unwrap();

        let panned = InspectionKey {
            zoom: Zoom::Percent(100),
            ..fit
        };
        assert!(inspection.request(panned, Arc::from(Path::new("image.png"))));
        assert!(!inspection.update_visible(Some(panned)));
        assert!(!inspection.update_visible(Some(panned)));
        assert!(!inspection.update_visible(None));
        assert!(inspection.update_visible(Some(panned)));
    }

    #[test]
    fn full_request_queue_does_not_cancel_accepted_work() {
        let (request_sender, request_receiver) = mpsc::sync_channel(1);
        let (_response_sender, response_receiver) = mpsc::sync_channel(1);
        let generation = Arc::new(AtomicU64::new(0));
        let mut inspection = Inspection {
            request_sender,
            response_receiver,
            generation: Arc::clone(&generation),
            schedule_gate: Arc::new(Mutex::new(())),
            desired: None,
            ready: None,
            error: None,
            visible_frame: None,
            next_frame_id: 0,
            response_closed: false,
        };
        let key = |entry_id| InspectionKey {
            entry_id: EntryId(entry_id),
            revision: SourceRevision::default(),
            size: Size::new(8, 4),
            zoom: Zoom::Fit,
            center_x: CENTER,
            center_y: CENTER,
            background: BackgroundMode::Dark,
            quality: "7".parse().unwrap(),
        };

        assert!(inspection.request(key(1), Arc::from(Path::new("first.png"))));
        assert!(!inspection.request(key(2), Arc::from(Path::new("second.png"))));
        assert_eq!(generation.load(Ordering::Acquire), 1);
        assert_eq!(inspection.desired, Some(key(1)));
        let accepted = request_receiver.try_recv().unwrap();
        assert_eq!(accepted.key, key(1));
        assert_eq!(accepted.generation, 1);
    }

    #[test]
    fn disconnected_inspection_worker_is_an_error_not_loading() {
        let (request_sender, request_receiver) = mpsc::sync_channel(1);
        drop(request_receiver);
        let (_response_sender, response_receiver) = mpsc::sync_channel(1);
        let key = InspectionKey {
            entry_id: EntryId(1),
            revision: SourceRevision::default(),
            size: Size::new(8, 4),
            zoom: Zoom::Fit,
            center_x: CENTER,
            center_y: CENTER,
            background: BackgroundMode::Dark,
            quality: "7".parse().unwrap(),
        };
        let mut inspection = Inspection {
            request_sender,
            response_receiver,
            generation: Arc::new(AtomicU64::new(0)),
            schedule_gate: Arc::new(Mutex::new(())),
            desired: None,
            ready: None,
            error: None,
            visible_frame: None,
            next_frame_id: 0,
            response_closed: false,
        };

        assert!(inspection.request(key, Arc::from(Path::new("image.png"))));
        let InspectionDisplay::Error(error) = inspection.display(key) else {
            panic!("disconnected worker must be visible as an error");
        };
        assert!(error.contains("unavailable"));
    }

    #[test]
    fn corrupt_source_becomes_a_recoverable_error_display() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "red-table-corrupt-inspection-{}-{nonce}.png",
            std::process::id()
        ));
        fs::write(&path, b"not an image").unwrap();
        let key = InspectionKey {
            entry_id: EntryId(1),
            revision: SourceRevision {
                size: 12,
                modified_nanoseconds: 1,
            },
            size: Size::new(8, 4),
            zoom: Zoom::Fit,
            center_x: CENTER,
            center_y: CENTER,
            background: BackgroundMode::Checkerboard,
            quality: "7".parse().unwrap(),
        };
        let mut inspection =
            Inspection::new(Picker::halfblocks(), DecodedSources::new(64 * 1024 * 1024));
        assert!(inspection.request(key, Arc::from(path.clone())));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            inspection.drain();
            if let InspectionDisplay::Error(error) = inspection.display(key) {
                assert!(!error.is_empty());
                break;
            }
            assert!(Instant::now() < deadline, "inspection worker timed out");
            thread::sleep(Duration::from_millis(5));
        }
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn comparison_workers_complete_valid_and_corrupt_sides_independently() {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "red-table-comparison-workers-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).unwrap();
        let valid_path = root.join("valid.png");
        let corrupt_path = root.join("corrupt.png");
        DynamicImage::new_rgb8(4, 4).save(&valid_path).unwrap();
        fs::write(&corrupt_path, b"not an image").unwrap();
        let key = |entry_id| InspectionKey {
            entry_id: EntryId(entry_id),
            revision: SourceRevision::default(),
            size: Size::new(8, 4),
            zoom: Zoom::Fit,
            center_x: CENTER,
            center_y: CENTER,
            background: BackgroundMode::Dark,
            quality: "7".parse().unwrap(),
        };
        let sources = DecodedSources::new(64 * 1024 * 1024);
        let mut left = Inspection::new(Picker::halfblocks(), sources.clone());
        let mut right = Inspection::new(Picker::halfblocks(), sources);
        assert!(left.request(key(1), Arc::from(valid_path)));
        assert!(right.request(key(2), Arc::from(corrupt_path)));
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            left.drain();
            right.drain();
            if matches!(left.display(key(1)), InspectionDisplay::Ready(_))
                && matches!(right.display(key(2)), InspectionDisplay::Error(_))
            {
                break;
            }
            assert!(Instant::now() < deadline, "comparison workers timed out");
            thread::sleep(Duration::from_millis(5));
        }
        fs::remove_dir_all(root).unwrap();
    }
}
