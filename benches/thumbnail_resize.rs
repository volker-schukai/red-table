use std::{hint::black_box, time::Instant};

use image::{DynamicImage, GenericImageView, Rgb, RgbImage, imageops::FilterType};

const SOURCE_WIDTH: u32 = 3840;
const SOURCE_HEIGHT: u32 = 2160;
const TARGET_WIDTH: u32 = 640;
const TARGET_HEIGHT: u32 = 360;
const HALF_TARGET_WIDTH: u32 = 32;
const HALF_TARGET_HEIGHT: u32 = 24;
const HALF_SOURCE_WIDTH: u32 = 128;
const HALF_SOURCE_HEIGHT: u32 = 72;
const SAMPLES: usize = 7;

fn main() {
    let source = representative_image();
    println!(
        "thumbnail resize: {SOURCE_WIDTH}x{SOURCE_HEIGHT} -> {TARGET_WIDTH}x{TARGET_HEIGHT}, median of {SAMPLES}"
    );
    for (label, filter, unsharpen) in [
        ("Q1", FilterType::Nearest, None),
        ("Q2", FilterType::Triangle, None),
        ("Q3", FilterType::CatmullRom, None),
        ("Q4", FilterType::Lanczos3, None),
        ("Q7", FilterType::Lanczos3, Some((0.8, 3))),
        ("Q9", FilterType::Lanczos3, Some((1.2, 1))),
    ] {
        let _ = resize(
            black_box(&source),
            TARGET_WIDTH,
            TARGET_HEIGHT,
            filter,
            unsharpen,
        );
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let started = Instant::now();
            let resized = resize(
                black_box(&source),
                TARGET_WIDTH,
                TARGET_HEIGHT,
                filter,
                unsharpen,
            );
            black_box(resized.get_pixel(TARGET_WIDTH / 2, TARGET_HEIGHT / 2));
            samples.push(started.elapsed());
        }
        samples.sort_unstable();
        println!("{label}: {:.2?}", samples[SAMPLES / 2]);
    }

    println!(
        "two-stage class reduction: {SOURCE_WIDTH}x{SOURCE_HEIGHT} -> 512 and 128, median of {SAMPLES}"
    );
    for (label, class) in [("x-large 512", 512), ("normal 128", 128)] {
        let direct = || source.resize(class, class, FilterType::Lanczos3);
        let staged = || {
            source
                .thumbnail(class * 2, class * 2)
                .resize(class, class, FilterType::Lanczos3)
        };
        println!("{label} direct Lanczos3: {:.2?}", median(direct));
        println!("{label} box prepass + Lanczos3: {:.2?}", median(staged));
    }

    let half_source =
        source.resize_exact(HALF_SOURCE_WIDTH, HALF_SOURCE_HEIGHT, FilterType::Lanczos3);
    println!(
        "halfblocks warm-cache grid: {HALF_SOURCE_WIDTH}x{HALF_SOURCE_HEIGHT} -> {HALF_TARGET_WIDTH}x{HALF_TARGET_HEIGHT}, median of {SAMPLES}"
    );
    for (label, filter, unsharpen) in [
        ("Q1", FilterType::Nearest, None),
        ("Q9", FilterType::Lanczos3, Some((1.2, 1))),
    ] {
        let _ = resize(
            black_box(&half_source),
            HALF_TARGET_WIDTH,
            HALF_TARGET_HEIGHT,
            filter,
            unsharpen,
        );
        let mut samples = Vec::with_capacity(SAMPLES);
        for _ in 0..SAMPLES {
            let started = Instant::now();
            let resized = resize(
                black_box(&half_source),
                HALF_TARGET_WIDTH,
                HALF_TARGET_HEIGHT,
                filter,
                unsharpen,
            );
            black_box(resized.get_pixel(HALF_TARGET_WIDTH / 2, HALF_TARGET_HEIGHT / 2));
            samples.push(started.elapsed());
        }
        samples.sort_unstable();
        println!("{label}: {:.2?}", samples[SAMPLES / 2]);
    }
}

fn median(operation: impl Fn() -> DynamicImage) -> std::time::Duration {
    let _ = operation();
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let prepared = black_box(operation());
        black_box(prepared.width());
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    samples[SAMPLES / 2]
}

fn resize(
    source: &DynamicImage,
    width: u32,
    height: u32,
    filter: FilterType,
    unsharpen: Option<(f32, i32)>,
) -> DynamicImage {
    let resized = source.resize(width, height, filter);
    match unsharpen {
        Some((sigma, threshold)) => resized.unsharpen(sigma, threshold),
        None => resized,
    }
}

fn representative_image() -> DynamicImage {
    RgbImage::from_fn(SOURCE_WIDTH, SOURCE_HEIGHT, |x, y| {
        let grain = ((x.wrapping_mul(17) ^ y.wrapping_mul(29)) & 31) as u8;
        let edge = if (x / 48 + y / 48).is_multiple_of(2) {
            48
        } else {
            0
        };
        Rgb([
            (x * 255 / SOURCE_WIDTH) as u8,
            (y * 255 / SOURCE_HEIGHT) as u8,
            64_u8.saturating_add(edge).saturating_add(grain),
        ])
    })
    .into()
}
