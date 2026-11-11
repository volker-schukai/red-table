use std::{
    cell::RefCell,
    path::Path,
    sync::{Condvar, Mutex, PoisonError},
};

use fast_image_resize::{FilterType as FastFilter, ResizeAlg, ResizeOptions, Resizer};
use image::{DynamicImage, ImageDecoder, Limits, imageops::FilterType, metadata::Orientation};

pub(crate) const MAX_DECODED_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
/// Process-wide bound on concurrently reserved decode memory.
const DECODE_BUDGET_BYTES: u64 = 2 * MAX_DECODED_SOURCE_BYTES;

static DECODE_BUDGET: DecodeBudget = DecodeBudget {
    used: Mutex::new(0),
    released: Condvar::new(),
};

thread_local! {
    /// Reservations held by the worker request running on this thread, or
    /// `None` outside [`with_decode_scope`].
    static SCOPED_RESERVATIONS: RefCell<Option<Vec<Reservation>>> = const { RefCell::new(None) };
}

struct DecodeBudget {
    used: Mutex<u64>,
    released: Condvar,
}

struct Reservation(u64);

impl Drop for Reservation {
    fn drop(&mut self) {
        let mut used = DECODE_BUDGET
            .used
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        *used -= self.0;
        DECODE_BUDGET.released.notify_all();
    }
}

/// Waits until `bytes` (clamped to the budget) fit the shared decode budget.
fn reserve(bytes: u64) -> Reservation {
    let bytes = bytes.min(DECODE_BUDGET_BYTES);
    let mut used = DECODE_BUDGET
        .used
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    while *used + bytes > DECODE_BUDGET_BYTES {
        used = DECODE_BUDGET
            .released
            .wait(used)
            .unwrap_or_else(PoisonError::into_inner);
    }
    *used += bytes;
    Reservation(bytes)
}

/// Decoded bytes to reserve, doubled for orientations that rotate into a
/// second buffer.
fn reservation_bytes(decoded: u64, orientation: Orientation) -> u64 {
    match orientation {
        Orientation::Rotate90
        | Orientation::Rotate270
        | Orientation::Rotate90FlipH
        | Orientation::Rotate270FlipH => decoded.saturating_mul(2),
        _ => decoded,
    }
}

/// Runs one worker request. Decode reservations made inside are held until it
/// returns or unwinds, covering the decoded source while it is downscaled.
pub(crate) fn with_decode_scope<T>(work: impl FnOnce() -> T) -> T {
    struct Scope;
    impl Drop for Scope {
        fn drop(&mut self) {
            let held = SCOPED_RESERVATIONS.with(RefCell::take);
            drop(held);
        }
    }
    SCOPED_RESERVATIONS.with(|held| *held.borrow_mut() = Some(Vec::new()));
    let _scope = Scope;
    work()
}
/// Sources larger than this multiple of the target are box-reduced first.
const PREPASS_RATIO: u32 = 2;

pub(crate) fn decode_oriented(path: &Path) -> Result<DynamicImage, String> {
    let mut reader = image::ImageReader::open(path)
        .and_then(image::ImageReader::with_guessed_format)
        .map_err(|error| error.to_string())?;
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_DECODED_SOURCE_BYTES);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().map_err(|error| error.to_string())?;
    if decoder.total_bytes() > MAX_DECODED_SOURCE_BYTES {
        return Err(format!(
            "decoded image exceeds the {} MiB source limit",
            MAX_DECODED_SOURCE_BYTES / (1024 * 1024)
        ));
    }
    let orientation = decoder.orientation().map_err(|error| error.to_string())?;
    // A retry within one request no longer needs its earlier source, and
    // holding it while waiting could deadlock against this request itself.
    let earlier = SCOPED_RESERVATIONS.with(|held| held.borrow_mut().as_mut().map(std::mem::take));
    drop(earlier);
    let reservation = reserve(reservation_bytes(decoder.total_bytes(), orientation));
    let mut image = DynamicImage::from_decoder(decoder).map_err(|error| error.to_string())?;
    image.apply_orientation(orientation);
    SCOPED_RESERVATIONS.with(|held| {
        if let Some(held) = held.borrow_mut().as_mut() {
            held.push(reservation);
        }
    });
    Ok(image)
}

/// Downscales `image` to fit `width` x `height` while preserving its aspect
/// ratio, exactly like `DynamicImage::resize`.
///
/// Sources more than twice as large as the target in either dimension are
/// first reduced with the box filter to twice the target bounds; the selected
/// `filter` then produces the final samples. This keeps the filter's edge
/// rendition at the final grid while removing the cost of running it over the
/// full source.
pub(crate) fn downscale(
    image: &DynamicImage,
    width: u32,
    height: u32,
    filter: FilterType,
) -> DynamicImage {
    let (target_width, target_height) =
        fit_dimensions(image.width(), image.height(), width, height);
    let prepass_width = target_width.saturating_mul(PREPASS_RATIO);
    let prepass_height = target_height.saturating_mul(PREPASS_RATIO);
    if image.width() > prepass_width || image.height() > prepass_height {
        let (reduced_width, reduced_height) =
            fit_dimensions(image.width(), image.height(), prepass_width, prepass_height);
        let reduced = convolve(image, None, reduced_width, reduced_height, FastFilter::Box)
            .unwrap_or_else(|| image.thumbnail(prepass_width, prepass_height));
        resize_exact(&reduced, target_width, target_height, filter)
    } else {
        resize_exact(image, target_width, target_height, filter)
    }
}

/// Resizes to exactly `width` x `height` samples like
/// `DynamicImage::resize_exact`, using SIMD convolution where the pixel layout
/// supports it.
pub(crate) fn resize_exact(
    image: &DynamicImage,
    width: u32,
    height: u32,
    filter: FilterType,
) -> DynamicImage {
    resample(image, None, width, height, filter)
        .unwrap_or_else(|| image.resize_exact(width, height, filter))
}

/// Resizes the `crop` region (`x`, `y`, `width`, `height`) of `image` to
/// exactly `width` x `height` samples without copying the region first.
pub(crate) fn resize_crop_exact(
    image: &DynamicImage,
    crop: (u32, u32, u32, u32),
    width: u32,
    height: u32,
    filter: FilterType,
) -> DynamicImage {
    resample(image, Some(crop), width, height, filter).unwrap_or_else(|| {
        image
            .crop_imm(crop.0, crop.1, crop.2, crop.3)
            .resize_exact(width, height, filter)
    })
}

fn resample(
    image: &DynamicImage,
    crop: Option<(u32, u32, u32, u32)>,
    width: u32,
    height: u32,
    filter: FilterType,
) -> Option<DynamicImage> {
    let filter = match filter {
        FilterType::Nearest => None,
        FilterType::Triangle => Some(FastFilter::Bilinear),
        FilterType::CatmullRom => Some(FastFilter::CatmullRom),
        FilterType::Gaussian => Some(FastFilter::Gaussian),
        FilterType::Lanczos3 => Some(FastFilter::Lanczos3),
    };
    let algorithm = filter.map_or(ResizeAlg::Nearest, ResizeAlg::Convolution);
    resample_with(image, crop, width, height, algorithm)
}

fn convolve(
    image: &DynamicImage,
    crop: Option<(u32, u32, u32, u32)>,
    width: u32,
    height: u32,
    filter: FastFilter,
) -> Option<DynamicImage> {
    resample_with(image, crop, width, height, ResizeAlg::Convolution(filter))
}

/// Returns `None` for empty targets and pixel layouts the SIMD resizer does not
/// support; callers then use the `image` crate.
fn resample_with(
    image: &DynamicImage,
    crop: Option<(u32, u32, u32, u32)>,
    width: u32,
    height: u32,
    algorithm: ResizeAlg,
) -> Option<DynamicImage> {
    thread_local! {
        /// Keeps the resizer's scratch buffers across requests of one worker.
        static RESIZER: RefCell<Resizer> = RefCell::new(Resizer::new());
    }
    if width == 0 || height == 0 {
        return None;
    }
    let mut options = ResizeOptions::new().resize_alg(algorithm);
    if let Some((x, y, crop_width, crop_height)) = crop {
        options = options.crop(
            f64::from(x),
            f64::from(y),
            f64::from(crop_width),
            f64::from(crop_height),
        );
    }
    let mut target = DynamicImage::new(width, height, image.color());
    RESIZER
        .with(|resizer| resizer.borrow_mut().resize(image, &mut target, &options))
        .ok()?;
    Some(target)
}

/// Mirrors the aspect-preserving fit the `image` crate applies in `resize`.
pub(crate) fn fit_dimensions(
    source_width: u32,
    source_height: u32,
    width: u32,
    height: u32,
) -> (u32, u32) {
    let width_ratio = f64::from(width) / f64::from(source_width.max(1));
    let height_ratio = f64::from(height) / f64::from(source_height.max(1));
    let ratio = width_ratio.min(height_ratio);
    let fitted_width = (f64::from(source_width) * ratio).round().max(1.0) as u32;
    let fitted_height = (f64::from(source_height) * ratio).round().max(1.0) as u32;
    (
        fitted_width.min(width.max(1)),
        fitted_height.min(height.max(1)),
    )
}

pub(crate) fn decoded_bytes(image: &DynamicImage) -> u64 {
    u64::from(image.width())
        .saturating_mul(u64::from(image.height()))
        .saturating_mul(u64::from(image.color().bytes_per_pixel()))
}

#[cfg(test)]
mod tests {
    use std::{fs::File, time::SystemTime};

    use image::{ExtendedColorType, ImageEncoder, Rgb, RgbImage, codecs::png::PngEncoder};

    use super::*;

    #[test]
    fn rotating_orientations_reserve_the_second_buffer() {
        assert_eq!(reservation_bytes(10, Orientation::NoTransforms), 10);
        assert_eq!(reservation_bytes(10, Orientation::Rotate180), 10);
        assert_eq!(reservation_bytes(10, Orientation::FlipHorizontal), 10);
        assert_eq!(reservation_bytes(10, Orientation::Rotate90), 20);
        assert_eq!(reservation_bytes(10, Orientation::Rotate270FlipH), 20);
        assert_eq!(reservation_bytes(u64::MAX, Orientation::Rotate90), u64::MAX);
    }

    #[test]
    fn decode_budget_blocks_until_reservations_are_released() {
        use std::{sync::mpsc, thread, time::Duration};

        // Clamped to the whole budget, so the waiter cannot fit beside it.
        let full = reserve(u64::MAX);
        let (sender, receiver) = mpsc::channel();
        let waiter = thread::spawn(move || {
            let reservation = reserve(1);
            sender.send(()).unwrap();
            drop(reservation);
        });
        assert!(
            receiver.recv_timeout(Duration::from_millis(50)).is_err(),
            "a full budget makes later decodes wait"
        );
        drop(full);
        receiver
            .recv_timeout(Duration::from_secs(10))
            .expect("released budget admits the waiter");
        waiter.join().unwrap();
    }

    #[test]
    fn scoped_reservations_live_until_the_request_ends() {
        let directory = std::env::temp_dir().join(format!(
            "red-table-budget-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("source.png");
        detailed_source(16, 8).save(&path).unwrap();
        let held = |scoped: bool| {
            let count = || SCOPED_RESERVATIONS.with(|held| held.borrow().as_ref().map(Vec::len));
            if scoped {
                with_decode_scope(|| {
                    decode_oriented(&path).unwrap();
                    decode_oriented(&path).unwrap();
                    count()
                })
            } else {
                decode_oriented(&path).unwrap();
                count()
            }
        };
        assert_eq!(held(true), Some(1), "a retry releases the earlier source");
        assert_eq!(held(false), None, "unscoped decodes release immediately");
        assert!(
            SCOPED_RESERVATIONS.with(|held| held.borrow().is_none()),
            "the scope clears its reservations"
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn detailed_source(width: u32, height: u32) -> DynamicImage {
        RgbImage::from_fn(width, height, |x, y| {
            let grain = ((x.wrapping_mul(17) ^ y.wrapping_mul(29)) & 31) as u8;
            let edge = if (x / 48 + y / 48).is_multiple_of(2) {
                48
            } else {
                0
            };
            Rgb([
                (x * 255 / width) as u8,
                (y * 255 / height) as u8,
                64_u8.saturating_add(edge).saturating_add(grain),
            ])
        })
        .into()
    }

    fn mean_absolute_difference(a: &DynamicImage, b: &DynamicImage) -> f64 {
        let (a, b) = (a.to_rgb8(), b.to_rgb8());
        assert_eq!(a.dimensions(), b.dimensions());
        let total: u64 = a
            .as_raw()
            .iter()
            .zip(b.as_raw())
            .map(|(left, right)| u64::from(left.abs_diff(*right)))
            .sum();
        total as f64 / a.as_raw().len() as f64
    }

    #[test]
    fn two_stage_downscale_matches_direct_dimensions_for_every_ratio() {
        for (width, height, target_width, target_height) in [
            (1920, 1080, 256, 256),
            (1080, 1920, 256, 256),
            (1000, 400, 128, 128),
            (300, 200, 128, 128),
            (257, 256, 128, 128),
            (1500, 1000, 320, 180),
            (7, 5, 3, 3),
        ] {
            let source = detailed_source(width, height);
            let direct = source.resize(target_width, target_height, FilterType::Lanczos3);
            let staged = downscale(&source, target_width, target_height, FilterType::Lanczos3);
            assert_eq!(
                (staged.width(), staged.height()),
                (direct.width(), direct.height()),
                "{width}x{height} -> {target_width}x{target_height}"
            );
        }
    }

    #[test]
    fn two_stage_downscale_stays_close_to_direct_filtering() {
        let source = detailed_source(1920, 1080);
        let direct = source.resize(256, 256, FilterType::Lanczos3);
        let staged = downscale(&source, 256, 256, FilterType::Lanczos3);
        let difference = mean_absolute_difference(&direct, &staged);
        assert!(
            difference < 3.0,
            "mean absolute channel difference {difference} exceeds the two-stage bound"
        );
        let nearest = source.resize(256, 256, FilterType::Nearest);
        assert!(
            difference < mean_absolute_difference(&direct, &nearest),
            "two-stage output must be closer to Lanczos3 than a Nearest resize"
        );
    }

    #[test]
    fn sources_within_the_prepass_ratio_use_the_direct_path() {
        let source = detailed_source(256, 144);
        let direct = source.resize(128, 128, FilterType::Lanczos3);
        let staged = downscale(&source, 128, 128, FilterType::Lanczos3);
        let difference = mean_absolute_difference(&direct, &staged);
        assert!(
            difference < 0.5,
            "single-stage SIMD output differs from the reference filter by {difference}"
        );
    }

    #[test]
    fn simd_resize_matches_the_reference_for_every_filter_and_layout() {
        let rgb = detailed_source(640, 360);
        for source in [
            rgb.clone(),
            DynamicImage::ImageRgba8(rgb.to_rgba8()),
            DynamicImage::ImageLuma8(rgb.to_luma8()),
            DynamicImage::ImageLumaA8(rgb.to_luma_alpha8()),
            DynamicImage::ImageRgb16(rgb.to_rgb16()),
        ] {
            for filter in [
                FilterType::Nearest,
                FilterType::Triangle,
                FilterType::CatmullRom,
                FilterType::Lanczos3,
            ] {
                for (width, height) in [(200, 120), (1000, 500)] {
                    let fast = resample(&source, None, width, height, filter)
                        .expect("common layouts use the SIMD path");
                    assert_eq!(fast.color(), source.color());
                    assert_eq!((fast.width(), fast.height()), (width, height));
                    let reference = source.resize_exact(width, height, filter);
                    let difference = mean_absolute_difference(&reference, &fast);
                    assert!(
                        difference < 2.0,
                        "{:?} {filter:?} -> {width}x{height}: {difference}",
                        source.color()
                    );
                }
            }
        }
    }

    #[test]
    fn cropped_resize_matches_cropping_first_and_empty_targets_fall_back() {
        let source = detailed_source(640, 360);
        let crop = (100, 50, 320, 200);
        let fast = resize_crop_exact(&source, crop, 640, 400, FilterType::Lanczos3);
        let reference = source
            .crop_imm(crop.0, crop.1, crop.2, crop.3)
            .resize_exact(640, 400, FilterType::Lanczos3);
        assert_eq!((fast.width(), fast.height()), (640, 400));
        assert!(mean_absolute_difference(&reference, &fast) < 1.0);

        assert!(resample(&source, None, 0, 10, FilterType::Lanczos3).is_none());
        assert_eq!(
            resize_exact(&source, 0, 10, FilterType::Lanczos3).width(),
            0
        );
    }

    fn temporary_png() -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "red-table-oriented-{}-{nonce}.png",
            std::process::id()
        ))
    }

    #[test]
    fn applies_exif_orientation_during_decode() {
        let path = temporary_png();
        let mut encoder = PngEncoder::new(File::create(&path).unwrap());
        // Little-endian TIFF IFD with Orientation=6 (90 degrees clockwise).
        encoder
            .set_exif_metadata(vec![
                0x49, 0x49, 0x2a, 0x00, 0x08, 0x00, 0x00, 0x00, 0x01, 0x00, 0x12, 0x01, 0x03, 0x00,
                0x01, 0x00, 0x00, 0x00, 0x06, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            ])
            .unwrap();
        encoder
            .write_image(
                &[255, 0, 0, 255, 0, 0, 255, 255],
                2,
                1,
                ExtendedColorType::Rgba8,
            )
            .unwrap();

        let image = decode_oriented(&path).unwrap();
        assert_eq!((image.width(), image.height()), (1, 2));
        assert_eq!(decoded_bytes(&image), 8);
        std::fs::remove_file(path).unwrap();
    }
}
