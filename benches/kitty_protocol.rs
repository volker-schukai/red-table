use std::{hint::black_box, time::Instant};

use image::{Rgb, RgbImage};
use ratatui::layout::Size;

#[allow(dead_code, unused_imports)]
#[path = "../source/kitty.rs"]
mod kitty;

use kitty::{KittyProtocol, KittySession};

const SOURCE_WIDTH: u32 = 640;
const SOURCE_HEIGHT: u32 = 360;
const CELL_WIDTH: u16 = 64;
const CELL_HEIGHT: u16 = 18;
const LARGE_WIDTH: u32 = 2000;
const LARGE_HEIGHT: u32 = 1000;
const SAMPLES: usize = 7;

fn main() {
    let source = RgbImage::from_fn(SOURCE_WIDTH, SOURCE_HEIGHT, |x, y| {
        let detail = ((x.wrapping_mul(31) ^ y.wrapping_mul(17)) & 63) as u8;
        Rgb([
            (x * 255 / SOURCE_WIDTH) as u8,
            (y * 255 / SOURCE_HEIGHT) as u8,
            48_u8.saturating_add(detail),
        ])
    });
    let size = Size::new(CELL_WIDTH, CELL_HEIGHT);
    let session = KittySession::with_tmux(false);
    let mut samples = Vec::with_capacity(SAMPLES);

    for _ in 0..SAMPLES {
        let started = Instant::now();
        let protocol = KittyProtocol::new(
            black_box(source.clone().into()),
            size,
            black_box(session.clone()),
        )
        .unwrap();
        black_box(protocol.size());
        samples.push(started.elapsed());
    }
    samples.sort_unstable();

    println!(
        "Kitty RGBA transport: {SOURCE_WIDTH}x{SOURCE_HEIGHT} over {CELL_WIDTH}x{CELL_HEIGHT} cells, median of {SAMPLES}"
    );
    println!("prepare: {:.2?}", samples[SAMPLES / 2]);

    let large = RgbImage::from_fn(LARGE_WIDTH, LARGE_HEIGHT, |x, y| {
        let detail = ((x.wrapping_mul(31) ^ y.wrapping_mul(17)) & 63) as u8;
        Rgb([
            (x * 255 / LARGE_WIDTH) as u8,
            (y * 255 / LARGE_HEIGHT) as u8,
            48_u8.saturating_add(detail),
        ])
    });
    let large_size = Size::new(200, 50);
    let mut samples = Vec::with_capacity(SAMPLES);
    let mut transmission_bytes = 0;
    for _ in 0..SAMPLES {
        let started = Instant::now();
        let protocol = KittyProtocol::new(
            black_box(large.clone().into()),
            large_size,
            black_box(session.clone()),
        )
        .unwrap();
        transmission_bytes = protocol.resident_bytes();
        samples.push(started.elapsed());
    }
    samples.sort_unstable();
    let raw_bytes = u64::from(LARGE_WIDTH) * u64::from(LARGE_HEIGHT) * 4;
    println!(
        "Kitty inspection transport (zlib above 1 MiB): {LARGE_WIDTH}x{LARGE_HEIGHT} over 200x50 cells, median of {SAMPLES}"
    );
    println!(
        "prepare: {:.2?}, raw RGBA {} MiB, transmission {} MiB",
        samples[SAMPLES / 2],
        raw_bytes.div_ceil(1024 * 1024),
        transmission_bytes.div_ceil(1024 * 1024)
    );
}
