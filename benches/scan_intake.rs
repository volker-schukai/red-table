use std::{
    fs,
    hint::black_box,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime},
};

#[allow(dead_code, unused_imports)]
#[path = "../source/app.rs"]
mod app;

#[allow(dead_code, unused_imports)]
#[path = "../source/scan.rs"]
mod scan;

use scan::ScanEvent;

const FILES: usize = 100_000;
const FILES_PER_DIRECTORY: usize = 1_000;
const FRAME: Duration = Duration::from_millis(33);
/// Mirrors the interface's per-frame scan drain budget.
const DRAIN_BUDGET: Duration = Duration::from_millis(8);

fn main() {
    let root = temporary_directory();
    for directory in 0..FILES / FILES_PER_DIRECTORY {
        let directory = root.join(format!("album-{directory:03}"));
        fs::create_dir_all(&directory).unwrap();
        for file in 0..FILES_PER_DIRECTORY {
            fs::write(directory.join(format!("image-{file:04}.jpg")), b"").unwrap();
        }
    }
    println!(
        "scan intake: {FILES} files in {} directories",
        FILES / FILES_PER_DIRECTORY
    );

    // Warm the directory cache so both measurements see the same filesystem.
    continuous(&root);
    let (walk, found) = continuous(&root);
    assert_eq!(found, FILES);
    println!(
        "continuous drain (walk speed): {walk:.2?}, {:.0} paths/s",
        FILES as f64 / walk.as_secs_f64()
    );
    let (frames, found) = frame_loop(&root);
    assert_eq!(found, FILES);
    println!("frame loop, {DRAIN_BUDGET:?} drain budget per {FRAME:?} frame: {frames:.2?}");
    fs::remove_dir_all(root).unwrap();
}

fn continuous(root: &Path) -> (Duration, usize) {
    let started = Instant::now();
    let mut found = 0;
    for event in scan::start(root.to_path_buf()) {
        match event {
            ScanEvent::Found(batch) => found += batch.len(),
            ScanEvent::Error => panic!("benchmark tree must be readable"),
            ScanEvent::Done => break,
        }
    }
    (started.elapsed(), found)
}

/// The interface loop: one budgeted drain, then the rest of the frame.
fn frame_loop(root: &Path) -> (Duration, usize) {
    let started = Instant::now();
    let receiver = scan::start(root.to_path_buf());
    let mut found = 0;
    loop {
        let frame_started = Instant::now();
        let drained = scan::drain(&receiver, DRAIN_BUDGET, |path, _| {
            black_box(path);
        });
        assert_eq!(drained.errors, 0, "benchmark tree must be readable");
        found += drained.found;
        if drained.done {
            return (started.elapsed(), found);
        }
        thread::sleep(FRAME.saturating_sub(frame_started.elapsed()));
    }
}

fn temporary_directory() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "red-table-scan-bench-{}-{nonce}",
        std::process::id()
    ))
}
