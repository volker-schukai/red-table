use std::{hint::black_box, time::Instant};

use image::{DynamicImage, GenericImageView, Rgb, RgbImage, imageops::FilterType};

const SOURCE_WIDTH: u32 = 6000;
const SOURCE_HEIGHT: u32 = 4000;
const TARGET_WIDTH: u32 = 1280;
const TARGET_HEIGHT: u32 = 720;
const SAMPLES: usize = 7;

fn main() {
    let source = representative_image();
    println!(
        "inspection preparation: {SOURCE_WIDTH}x{SOURCE_HEIGHT} -> {TARGET_WIDTH}x{TARGET_HEIGHT}, median of {SAMPLES}"
    );
    measure("fit direct Lanczos3", || {
        source.resize(TARGET_WIDTH, TARGET_HEIGHT, FilterType::Lanczos3)
    });
    measure("fit box prepass + Lanczos3", || {
        source
            .thumbnail(TARGET_WIDTH * 2, TARGET_HEIGHT * 2)
            .resize(TARGET_WIDTH, TARGET_HEIGHT, FilterType::Lanczos3)
    });
    measure("100% centered crop", || {
        let x = (SOURCE_WIDTH - TARGET_WIDTH) / 2;
        let y = (SOURCE_HEIGHT - TARGET_HEIGHT) / 2;
        source
            .crop_imm(x, y, TARGET_WIDTH, TARGET_HEIGHT)
            .resize_exact(TARGET_WIDTH, TARGET_HEIGHT, FilterType::Lanczos3)
    });
}

fn measure(label: &str, operation: impl Fn() -> DynamicImage) {
    let _ = operation();
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let prepared = black_box(operation());
        black_box(prepared.get_pixel(TARGET_WIDTH / 2, TARGET_HEIGHT / 2));
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    println!("{label}: {:.2?}", samples[SAMPLES / 2]);
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
