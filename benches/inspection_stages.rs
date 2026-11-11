use std::{
    hint::black_box,
    io::Write,
    time::{Duration, Instant},
};

use flate2::{Compression, write::ZlibEncoder};
use image::{DynamicImage, Rgb, RgbImage, Rgba, RgbaImage, imageops::FilterType};

#[allow(dead_code, unused_imports)]
#[path = "../source/image_source.rs"]
mod image_source;

const SAMPLES: usize = 7;
const VIEW_WIDTH: u32 = 2560;
const VIEW_HEIGHT: u32 = 1400;

fn main() {
    let source = camera_like_image(6000, 4000);
    // 200% zoom: a 1280x700 source region fills the view.
    let crop = (2360, 1650, 1280, 700);
    println!(
        "inspection step stages: 6000x4000 source, 200% zoom into {VIEW_WIDTH}x{VIEW_HEIGHT}, median of {SAMPLES}"
    );

    let rendered = stage("resample crop (SIMD Lanczos3)", || {
        image_source::resize_crop_exact(
            &source,
            crop,
            VIEW_WIDTH,
            VIEW_HEIGHT,
            FilterType::Lanczos3,
        )
    });
    let canvas = stage("checkerboard RGBA canvas", checkerboard_rgba);
    let composite = stage("blend onto RGBA canvas", || {
        let mut canvas = canvas.clone();
        image::imageops::overlay(&mut canvas, &rendered.to_rgba8(), 0, 0);
        canvas
    });
    let rgb = stage("RGBA to RGB", || {
        DynamicImage::ImageRgba8(composite.clone()).into_rgb8()
    });
    stage("opaque path: copy rows into RGB canvas", || {
        let mut canvas = RgbImage::new(VIEW_WIDTH, VIEW_HEIGHT);
        image::imageops::replace(&mut canvas, rendered.as_rgb8().unwrap(), 0, 0);
        canvas
    });
    let compressed = stage("zlib fastest level", || {
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(rgb.as_raw()).unwrap();
        encoder.finish().unwrap()
    });
    println!(
        "  raw {} KiB -> compressed {} KiB",
        rgb.as_raw().len() / 1024,
        compressed.len() / 1024
    );
    stage("Base64", || {
        let mut output = String::new();
        base64_simd::STANDARD.encode_append(&compressed, &mut output);
        output
    });
}

fn stage<T>(label: &str, operation: impl Fn() -> T) -> T {
    let mut output = operation();
    let mut times = Vec::with_capacity(SAMPLES);
    for _ in 0..SAMPLES {
        let started = Instant::now();
        output = black_box(operation());
        times.push(started.elapsed());
    }
    times.sort_unstable();
    let median: Duration = times[SAMPLES / 2];
    println!("{label}: {median:.2?}");
    output
}

fn checkerboard_rgba() -> RgbaImage {
    RgbaImage::from_fn(VIEW_WIDTH, VIEW_HEIGHT, |x, y| {
        let value = if (x / 8 + y / 8).is_multiple_of(2) {
            72
        } else {
            112
        };
        Rgba([value, value, value, 255])
    })
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
