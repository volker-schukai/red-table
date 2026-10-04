use std::path::Path;

use image::{DynamicImage, ImageDecoder, Limits, imageops::FilterType};

pub(crate) const MAX_DECODED_SOURCE_BYTES: u64 = 512 * 1024 * 1024;
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
    let mut image = DynamicImage::from_decoder(decoder).map_err(|error| error.to_string())?;
    image.apply_orientation(orientation);
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
        image
            .thumbnail(prepass_width, prepass_height)
            .resize(target_width, target_height, filter)
    } else {
        image.resize(target_width, target_height, filter)
    }
}

/// Mirrors the aspect-preserving fit the `image` crate applies in `resize`.
fn fit_dimensions(source_width: u32, source_height: u32, width: u32, height: u32) -> (u32, u32) {
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
        assert_eq!(mean_absolute_difference(&direct, &staged), 0.0);
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
