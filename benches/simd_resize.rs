use std::{
    hint::black_box,
    time::{Duration, Instant},
};

use image::{DynamicImage, Rgb, RgbImage, imageops::FilterType};

#[allow(dead_code, unused_imports)]
#[path = "../source/image_source.rs"]
mod image_source;

const SAMPLES: usize = 7;

fn main() {
    let camera = camera_like_image(6000, 4000);
    let class = image_source::downscale(&camera, 512, 512, FilterType::Lanczos3);
    println!(
        "resize: `image` crate reference vs shared SIMD resize, median of {SAMPLES}, interleaved"
    );

    for dimension in [128, 512] {
        compare(
            &format!("class reduction 6000x4000 -> {dimension} box"),
            || reference_downscale(&camera, dimension, dimension),
            || image_source::downscale(&camera, dimension, dimension, FilterType::Lanczos3),
        );
    }
    compare(
        "final grid 512x341 -> 280x187",
        || class.resize_exact(280, 187, FilterType::Lanczos3),
        || image_source::resize_exact(&class, 280, 187, FilterType::Lanczos3),
    );
    compare(
        "inspection fit 6000x4000 -> 2100x1400",
        || reference_downscale(&camera, 2100, 1400),
        || image_source::downscale(&camera, 2100, 1400, FilterType::Lanczos3),
    );
    for (label, crop) in [
        ("50% crop 5120x2800", (400, 600, 5120, 2800)),
        ("200% crop 1280x700", (2360, 1650, 1280, 700)),
    ] {
        compare(
            &format!("inspection {label} -> 2560x1400"),
            || {
                camera
                    .crop_imm(crop.0, crop.1, crop.2, crop.3)
                    .resize_exact(2560, 1400, FilterType::Lanczos3)
            },
            || image_source::resize_crop_exact(&camera, crop, 2560, 1400, FilterType::Lanczos3),
        );
    }
}

/// The two-stage `image` crate downscale used before the SIMD resize.
fn reference_downscale(image: &DynamicImage, width: u32, height: u32) -> DynamicImage {
    let (target_width, target_height) =
        image_source::fit_dimensions(image.width(), image.height(), width, height);
    if image.width() > target_width * 2 || image.height() > target_height * 2 {
        image.thumbnail(target_width * 2, target_height * 2).resize(
            target_width,
            target_height,
            FilterType::Lanczos3,
        )
    } else {
        image.resize(target_width, target_height, FilterType::Lanczos3)
    }
}

fn compare(label: &str, current: impl Fn() -> DynamicImage, candidate: impl Fn() -> DynamicImage) {
    let (mut current_times, mut candidate_times) = (Vec::new(), Vec::new());
    let (mut current_image, mut candidate_image) = (current(), candidate());
    for _ in 0..SAMPLES {
        let started = Instant::now();
        current_image = black_box(current());
        current_times.push(started.elapsed());
        let started = Instant::now();
        candidate_image = black_box(candidate());
        candidate_times.push(started.elapsed());
    }
    let (current_time, candidate_time) = (median(current_times), median(candidate_times));
    println!(
        "{label}: reference {current_time:.2?}, SIMD {candidate_time:.2?} ({:.1}x), PSNR {:.1} dB",
        current_time.as_secs_f64() / candidate_time.as_secs_f64(),
        psnr(&current_image, &candidate_image)
    );
}

fn median(mut times: Vec<Duration>) -> Duration {
    times.sort_unstable();
    times[times.len() / 2]
}

fn psnr(left: &DynamicImage, right: &DynamicImage) -> f64 {
    let (left, right) = (left.to_rgb8(), right.to_rgb8());
    assert_eq!(left.dimensions(), right.dimensions());
    let squared: f64 = left
        .as_raw()
        .iter()
        .zip(right.as_raw())
        .map(|(a, b)| (f64::from(*a) - f64::from(*b)).powi(2))
        .sum();
    let mse = squared / left.as_raw().len() as f64;
    if mse == 0.0 {
        f64::INFINITY
    } else {
        10.0 * (255.0_f64.powi(2) / mse).log10()
    }
}

/// Smooth gradients with fine texture and hard edges, like a photograph.
fn camera_like_image(width: u32, height: u32) -> DynamicImage {
    let mut state = 0x2545_F491_u32;
    RgbImage::from_fn(width, height, |x, y| {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        let noise = (state & 15) as f32 - 7.5;
        let fx = x as f32 / width as f32;
        let fy = y as f32 / height as f32;
        let wave = (fx * 40.0).sin() * (fy * 25.0).cos() * 40.0;
        let edge = if (x / 300 + y / 300).is_multiple_of(2) {
            30.0
        } else {
            -30.0
        };
        let channel = |base: f32| (base + wave + edge + noise).clamp(0.0, 255.0) as u8;
        Rgb([
            channel(60.0 + fx * 140.0),
            channel(80.0 + fy * 120.0),
            channel(150.0 - fx * 60.0),
        ])
    })
    .into()
}
