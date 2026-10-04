use std::{
    collections::HashMap,
    env,
    fs::{self, File, OpenOptions},
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::UNIX_EPOCH,
};

use image::{DynamicImage, GrayAlphaImage, GrayImage, RgbImage, RgbaImage, imageops::FilterType};
use url::Url;

#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

const CACHE_SCHEMA: &str = "2";
const MAX_SOURCE_ATTEMPTS: usize = 2;
const MAX_DECODED_CACHE_BYTES: usize = 8 * 1024 * 1024;
static TEMPORARY_SEQUENCE: AtomicU64 = AtomicU64::new(0);

const THUMB_URI: &str = "Thumb::URI";
const THUMB_MTIME: &str = "Thumb::MTime";
const THUMB_SIZE: &str = "Thumb::Size";
const RED_TABLE_MTIME_NSEC: &str = "X-RedTable::MTimeNsec";
const RED_TABLE_SCHEMA: &str = "X-RedTable::Schema";

#[derive(Clone, Debug)]
pub(crate) struct PersistentCache {
    root: Option<Arc<PathBuf>>,
}

#[derive(Debug)]
pub(crate) struct PreparedSource {
    pub(crate) image: DynamicImage,
    pub(crate) disk_hit: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CacheClass {
    Normal,
    Large,
    ExtraLarge,
    ExtraExtraLarge,
}

impl CacheClass {
    fn for_required_pixels(width: u32, height: u32) -> Option<Self> {
        match width.max(height) {
            0..=128 => Some(Self::Normal),
            129..=256 => Some(Self::Large),
            257..=512 => Some(Self::ExtraLarge),
            513..=1024 => Some(Self::ExtraExtraLarge),
            _ => None,
        }
    }

    const fn directory(self) -> &'static str {
        match self {
            Self::Normal => "normal",
            Self::Large => "large",
            Self::ExtraLarge => "x-large",
            Self::ExtraExtraLarge => "xx-large",
        }
    }

    const fn dimension(self) -> u32 {
        match self {
            Self::Normal => 128,
            Self::Large => 256,
            Self::ExtraLarge => 512,
            Self::ExtraExtraLarge => 1024,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceStamp {
    uri: String,
    size: u64,
    mtime_seconds: u64,
    mtime_nanoseconds: u128,
}

impl SourceStamp {
    fn read(path: &Path) -> Result<Self, String> {
        let canonical = path.canonicalize().map_err(|error| error.to_string())?;
        let metadata = canonical.metadata().map_err(|error| error.to_string())?;
        if !metadata.is_file() {
            return Err(format!("not a regular file: {}", canonical.display()));
        }
        let modified = metadata.modified().map_err(|error| error.to_string())?;
        let since_epoch = modified
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "source modification time predates the Unix epoch")?;
        let uri = Url::from_file_path(&canonical)
            .map_err(|_| format!("cannot create file URI for {}", canonical.display()))?
            .into();
        Ok(Self {
            uri,
            size: metadata.len(),
            mtime_seconds: since_epoch.as_secs(),
            mtime_nanoseconds: since_epoch.as_nanos(),
        })
    }
}

#[derive(Debug)]
struct CacheLocation {
    directory: PathBuf,
    path: PathBuf,
    class: CacheClass,
}

impl PersistentCache {
    pub(crate) fn discover() -> Self {
        let cache_home = env::var_os("XDG_CACHE_HOME")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")));
        match cache_home {
            Some(home) => Self::with_root(home.join("thumbnails")),
            None => Self::disabled(),
        }
    }

    pub(crate) const fn disabled() -> Self {
        Self { root: None }
    }

    pub(crate) fn with_root(root: PathBuf) -> Self {
        Self {
            root: Some(Arc::new(root)),
        }
    }

    pub(crate) fn prepare(
        &self,
        path: &Path,
        required_pixels: (u32, u32),
    ) -> Result<PreparedSource, String> {
        self.prepare_with(path, required_pixels, crate::image_source::decode_oriented)
    }

    fn prepare_with<F>(
        &self,
        path: &Path,
        required_pixels: (u32, u32),
        mut decode: F,
    ) -> Result<PreparedSource, String>
    where
        F: FnMut(&Path) -> Result<DynamicImage, String>,
    {
        let Some(class) = CacheClass::for_required_pixels(required_pixels.0, required_pixels.1)
        else {
            return decode(path).map(|image| PreparedSource {
                image,
                disk_hit: false,
            });
        };
        let Some(root) = self.root.as_deref() else {
            return decode(path).map(|image| PreparedSource {
                image,
                disk_hit: false,
            });
        };

        for _ in 0..MAX_SOURCE_ATTEMPTS {
            let before = match SourceStamp::read(path) {
                Ok(stamp) => stamp,
                Err(_) => {
                    return decode(path).map(|image| PreparedSource {
                        image,
                        disk_hit: false,
                    });
                }
            };
            let location = cache_location(root, class, &before.uri);
            if let Ok(Some(image)) = read_valid_cache(&location, &before)
                && SourceStamp::read(path).is_ok_and(|after| after == before)
            {
                return Ok(PreparedSource {
                    image,
                    disk_hit: true,
                });
            }

            let original = decode(path)?;
            let persistent = resize_for_class(original, class);
            if SourceStamp::read(path).is_ok_and(|after| after == before) {
                let _ = write_cache(&location, &before, &persistent);
                return Ok(PreparedSource {
                    image: persistent,
                    disk_hit: false,
                });
            }
        }

        Err(format!(
            "source changed repeatedly while generating thumbnail: {}",
            path.display()
        ))
    }
}

fn resize_for_class(image: DynamicImage, class: CacheClass) -> DynamicImage {
    let dimension = class.dimension();
    if image.width() <= dimension && image.height() <= dimension {
        image
    } else {
        crate::image_source::downscale(&image, dimension, dimension, FilterType::Lanczos3)
    }
}

fn cache_location(root: &Path, class: CacheClass, uri: &str) -> CacheLocation {
    let directory = root.join(class.directory());
    let filename = format!("{:x}.png", md5::compute(uri.as_bytes()));
    CacheLocation {
        path: directory.join(filename),
        directory,
        class,
    }
}

fn read_valid_cache(
    location: &CacheLocation,
    source: &SourceStamp,
) -> Result<Option<DynamicImage>, String> {
    let file = match File::open(&location.path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_limits(png::Limits {
        bytes: MAX_DECODED_CACHE_BYTES,
    });
    decoder.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    let info = reader.info();
    if info.width == 0
        || info.height == 0
        || info.width > location.class.dimension()
        || info.height > location.class.dimension()
    {
        return Ok(None);
    }
    let text = text_metadata(info);
    if !metadata_is_current(&text, source) {
        return Ok(None);
    }

    let buffer_size = reader
        .output_buffer_size()
        .ok_or("cached PNG exceeds the decode limit")?;
    let mut pixels = vec![0; buffer_size];
    let output = reader
        .next_frame(&mut pixels)
        .map_err(|error| error.to_string())?;
    pixels.truncate(output.buffer_size());
    decode_pixels(output, pixels).map(Some)
}

fn text_metadata(info: &png::Info<'_>) -> HashMap<String, String> {
    let mut fields = HashMap::new();
    for chunk in &info.uncompressed_latin1_text {
        fields.insert(chunk.keyword.clone(), chunk.text.clone());
    }
    for chunk in &info.compressed_latin1_text {
        if let Ok(text) = chunk.get_text() {
            fields.insert(chunk.keyword.clone(), text);
        }
    }
    for chunk in &info.utf8_text {
        if let Ok(text) = chunk.get_text() {
            fields.insert(chunk.keyword.clone(), text);
        }
    }
    fields
}

fn metadata_is_current(fields: &HashMap<String, String>, source: &SourceStamp) -> bool {
    if fields.get(THUMB_URI) != Some(&source.uri)
        || fields.get(THUMB_MTIME) != Some(&source.mtime_seconds.to_string())
        || fields.get(THUMB_SIZE) != Some(&source.size.to_string())
    {
        return false;
    }

    match fields.get(RED_TABLE_SCHEMA) {
        Some(schema) => {
            schema == CACHE_SCHEMA
                && fields.get(RED_TABLE_MTIME_NSEC) == Some(&source.mtime_nanoseconds.to_string())
        }
        None => true,
    }
}

fn decode_pixels(output: png::OutputInfo, pixels: Vec<u8>) -> Result<DynamicImage, String> {
    let dimensions = (output.width, output.height);
    match output.color_type {
        png::ColorType::Grayscale => {
            GrayImage::from_raw(dimensions.0, dimensions.1, pixels).map(DynamicImage::ImageLuma8)
        }
        png::ColorType::GrayscaleAlpha => {
            GrayAlphaImage::from_raw(dimensions.0, dimensions.1, pixels)
                .map(DynamicImage::ImageLumaA8)
        }
        png::ColorType::Rgb => {
            RgbImage::from_raw(dimensions.0, dimensions.1, pixels).map(DynamicImage::ImageRgb8)
        }
        png::ColorType::Rgba => {
            RgbaImage::from_raw(dimensions.0, dimensions.1, pixels).map(DynamicImage::ImageRgba8)
        }
        png::ColorType::Indexed => None,
    }
    .ok_or_else(|| "cached PNG pixel buffer is inconsistent".into())
}

fn write_cache(
    location: &CacheLocation,
    source: &SourceStamp,
    image: &DynamicImage,
) -> Result<(), String> {
    create_private_directory(&location.directory)?;
    let temporary = location.directory.join(format!(
        ".red-table-{}-{}.tmp",
        std::process::id(),
        TEMPORARY_SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));

    let result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = options
            .open(&temporary)
            .map_err(|error| error.to_string())?;
        let mut buffered = BufWriter::new(file);
        {
            let rgba = image.to_rgba8();
            let mut encoder = png::Encoder::new(&mut buffered, rgba.width(), rgba.height());
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            for (key, value) in [
                (THUMB_URI, source.uri.as_str()),
                (THUMB_MTIME, &source.mtime_seconds.to_string()),
                (THUMB_SIZE, &source.size.to_string()),
                (RED_TABLE_MTIME_NSEC, &source.mtime_nanoseconds.to_string()),
                (RED_TABLE_SCHEMA, CACHE_SCHEMA),
            ] {
                encoder
                    .add_text_chunk(key.to_owned(), value.to_owned())
                    .map_err(|error| error.to_string())?;
            }
            let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
            writer
                .write_image_data(rgba.as_raw())
                .map_err(|error| error.to_string())?;
            writer.finish().map_err(|error| error.to_string())?;
        }
        buffered.flush().map_err(|error| error.to_string())?;
        drop(buffered);
        // Publication is the atomic rename; the cache is regenerable, so no
        // fsync is issued for the file or the directory.
        fs::rename(&temporary, &location.path).map_err(|error| error.to_string())?;
        #[cfg(unix)]
        fs::set_permissions(&location.path, fs::Permissions::from_mode(0o600))
            .map_err(|error| error.to_string())?;
        Ok(())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn create_private_directory(directory: &Path) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        if let Some(root) = directory.parent() {
            fs::set_permissions(root, fs::Permissions::from_mode(0o700))
                .map_err(|error| error.to_string())?;
        }
        fs::set_permissions(directory, fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, Barrier},
        thread,
        time::SystemTime,
    };

    use image::{GenericImageView, Rgb, RgbImage};

    use super::*;

    fn temporary_directory(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "red-table-cache-{name}-{}-{nonce}",
            std::process::id()
        ))
    }

    fn source_image(path: &Path, width: u32, height: u32, color: [u8; 3]) {
        RgbImage::from_pixel(width, height, Rgb(color))
            .save(path)
            .unwrap();
    }

    #[test]
    fn selects_standard_size_classes() {
        assert_eq!(
            CacheClass::for_required_pixels(128, 40),
            Some(CacheClass::Normal)
        );
        assert_eq!(
            CacheClass::for_required_pixels(129, 40),
            Some(CacheClass::Large)
        );
        assert_eq!(
            CacheClass::for_required_pixels(500, 512),
            Some(CacheClass::ExtraLarge)
        );
        assert_eq!(
            CacheClass::for_required_pixels(513, 20),
            Some(CacheClass::ExtraExtraLarge)
        );
        assert_eq!(CacheClass::for_required_pixels(1025, 20), None);
    }

    #[test]
    fn uses_the_standard_uri_digest_filename() {
        let root = Path::new("/cache/thumbnails");
        let location = cache_location(root, CacheClass::Normal, "file:///home/jens/photos/me.png");
        assert_eq!(
            location.path,
            root.join("normal/c6ee772d9e49320e97ec29a7eb5b1697.png")
        );
    }

    #[test]
    fn validates_standard_and_red_table_metadata_contracts() {
        let source = SourceStamp {
            uri: "file:///photos/source.png".into(),
            size: 42,
            mtime_seconds: 10,
            mtime_nanoseconds: 10_000_000_123,
        };
        let mut fields = HashMap::from([
            (THUMB_URI.into(), source.uri.clone()),
            (THUMB_MTIME.into(), source.mtime_seconds.to_string()),
            (THUMB_SIZE.into(), source.size.to_string()),
        ]);

        assert!(metadata_is_current(&fields, &source));
        fields.insert(RED_TABLE_SCHEMA.into(), CACHE_SCHEMA.into());
        fields.insert(
            RED_TABLE_MTIME_NSEC.into(),
            source.mtime_nanoseconds.to_string(),
        );
        assert!(metadata_is_current(&fields, &source));

        fields.insert(RED_TABLE_SCHEMA.into(), "1".into());
        assert!(!metadata_is_current(&fields, &source));
        fields.insert(RED_TABLE_SCHEMA.into(), CACHE_SCHEMA.into());
        fields.insert(RED_TABLE_MTIME_NSEC.into(), "10000000124".into());
        assert!(!metadata_is_current(&fields, &source));
        fields.remove(RED_TABLE_SCHEMA);
        fields.remove(RED_TABLE_MTIME_NSEC);
        fields.remove(THUMB_SIZE);
        assert!(!metadata_is_current(&fields, &source));
    }

    #[test]
    fn bypasses_persistence_above_the_largest_standard_class() {
        let directory = temporary_directory("oversize");
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.png");
        source_image(&source, 32, 32, [20, 80, 160]);
        let cache_root = directory.join("cache/thumbnails");
        let cache = PersistentCache::with_root(cache_root.clone());

        assert!(!cache.prepare(&source, (1025, 20)).unwrap().disk_hit);
        assert!(!cache.prepare(&source, (1025, 20)).unwrap().disk_hit);
        assert!(!cache_root.exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn warm_cache_avoids_original_decode_and_carries_required_metadata() {
        let directory = temporary_directory("warm");
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.png");
        source_image(&source, 640, 480, [20, 80, 160]);
        let cache = PersistentCache::with_root(directory.join("cache/thumbnails"));
        let mut decodes = 0;
        let cold = cache
            .prepare_with(&source, (200, 120), |path| {
                decodes += 1;
                crate::image_source::decode_oriented(path)
            })
            .unwrap();
        assert!(!cold.disk_hit);
        assert_eq!(decodes, 1);

        let warm = cache
            .prepare_with(&source, (200, 120), |_| {
                decodes += 1;
                Err("warm hit must not decode the original".into())
            })
            .unwrap();
        assert!(warm.disk_hit);
        assert_eq!(decodes, 1);

        let stamp = SourceStamp::read(&source).unwrap();
        let location = cache_location(
            cache.root.as_deref().unwrap(),
            CacheClass::Large,
            &stamp.uri,
        );
        let file = File::open(location.path).unwrap();
        let reader = png::Decoder::new(BufReader::new(file)).read_info().unwrap();
        let fields = text_metadata(reader.info());
        assert!(metadata_is_current(&fields, &stamp));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn source_replacement_invalidates_a_warm_entry() {
        let directory = temporary_directory("invalidate");
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.png");
        source_image(&source, 640, 480, [20, 80, 160]);
        let cache = PersistentCache::with_root(directory.join("cache/thumbnails"));
        cache.prepare(&source, (120, 80)).unwrap();

        source_image(&source, 801, 601, [180, 10, 40]);
        let refreshed = cache.prepare(&source, (120, 80)).unwrap();
        assert!(!refreshed.disk_hit);
        assert_eq!(refreshed.image.get_pixel(0, 0).0[..3], [180, 10, 40]);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn corrupt_cache_regenerates_without_failing() {
        let directory = temporary_directory("corrupt");
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.png");
        source_image(&source, 320, 240, [40, 90, 130]);
        let cache = PersistentCache::with_root(directory.join("cache/thumbnails"));
        cache.prepare(&source, (100, 80)).unwrap();
        let stamp = SourceStamp::read(&source).unwrap();
        let location = cache_location(
            cache.root.as_deref().unwrap(),
            CacheClass::Normal,
            &stamp.uri,
        );
        fs::write(&location.path, b"broken PNG").unwrap();

        let regenerated = cache.prepare(&source, (100, 80)).unwrap();
        assert!(!regenerated.disk_hit);
        assert!(read_valid_cache(&location, &stamp).unwrap().is_some());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn publication_reads_only_the_digest_path_and_leaves_no_temporaries() {
        let directory = temporary_directory("publish");
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.png");
        source_image(&source, 320, 240, [40, 90, 130]);
        let cache = PersistentCache::with_root(directory.join("cache/thumbnails"));
        let stamp = SourceStamp::read(&source).unwrap();
        let location = cache_location(
            cache.root.as_deref().unwrap(),
            CacheClass::Normal,
            &stamp.uri,
        );
        fs::create_dir_all(&location.directory).unwrap();
        fs::write(
            location.directory.join(".red-table-1-1.tmp"),
            b"an interrupted writer left this behind",
        )
        .unwrap();

        assert!(read_valid_cache(&location, &stamp).unwrap().is_none());
        let prepared = cache.prepare(&source, (100, 80)).unwrap();
        assert!(!prepared.disk_hit);
        assert!(read_valid_cache(&location, &stamp).unwrap().is_some());
        let names = fs::read_dir(&location.directory)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(names.len(), 2, "{names:?}");
        assert!(names.iter().any(|name| name == ".red-table-1-1.tmp"));
        assert!(names.iter().any(|name| name.ends_with(".png")));
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn two_stage_class_reduction_matches_direct_dimensions() {
        let source = DynamicImage::ImageRgb8(RgbImage::from_fn(1600, 900, |x, y| {
            Rgb([(x % 256) as u8, (y % 256) as u8, ((x ^ y) & 255) as u8])
        }));
        let reduced = resize_for_class(source.clone(), CacheClass::Normal);
        assert_eq!((reduced.width(), reduced.height()), (128, 72));
        let direct = source.resize(128, 128, FilterType::Lanczos3);
        assert_eq!((direct.width(), direct.height()), (128, 72));
        let small = resize_for_class(
            source.resize(100, 100, FilterType::Triangle),
            CacheClass::Normal,
        );
        assert_eq!((small.width(), small.height()), (100, 56));
    }

    #[test]
    fn source_race_discards_the_first_generation() {
        let directory = temporary_directory("race");
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.png");
        source_image(&source, 320, 240, [10, 20, 30]);
        let cache = PersistentCache::with_root(directory.join("cache/thumbnails"));
        let mut calls = 0;

        let prepared = cache
            .prepare_with(&source, (100, 80), |path| {
                calls += 1;
                let decoded = crate::image_source::decode_oriented(path)?;
                if calls == 1 {
                    source_image(path, 401, 301, [200, 100, 50]);
                }
                Ok(decoded)
            })
            .unwrap();

        assert_eq!(calls, 2);
        assert_eq!(prepared.image.get_pixel(0, 0).0[..3], [200, 100, 50]);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn concurrent_writers_publish_one_complete_private_png() {
        let directory = temporary_directory("concurrent");
        fs::create_dir_all(&directory).unwrap();
        let source = directory.join("source.png");
        source_image(&source, 640, 480, [50, 100, 150]);
        let cache = PersistentCache::with_root(directory.join("cache/thumbnails"));
        let barrier = Arc::new(Barrier::new(4));
        let mut workers = Vec::new();
        for _ in 0..4 {
            let cache = cache.clone();
            let source = source.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                cache.prepare(&source, (200, 120)).unwrap();
            }));
        }
        for worker in workers {
            worker.join().unwrap();
        }

        let stamp = SourceStamp::read(&source).unwrap();
        let location = cache_location(
            cache.root.as_deref().unwrap(),
            CacheClass::Large,
            &stamp.uri,
        );
        assert!(read_valid_cache(&location, &stamp).unwrap().is_some());
        #[cfg(unix)]
        {
            assert_eq!(
                fs::metadata(&location.directory)
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(&location.path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
        assert!(fs::read_dir(&location.directory).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .ends_with(".tmp")
        }));
        fs::remove_dir_all(directory).unwrap();
    }
}
