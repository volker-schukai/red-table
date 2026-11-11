#[cfg(test)]
use std::time::SystemTime;
use std::{
    collections::HashSet,
    path::PathBuf,
    sync::mpsc,
    thread,
    time::{Duration, Instant, UNIX_EPOCH},
};

use ignore::{WalkBuilder, WalkState};

use crate::app::{SourceRevision, is_supported_image};

/// Found images per scan event; with [`CHANNEL_BATCHES`] this bounds the paths
/// buffered between the scanner and the interface.
const BATCH_ENTRIES: usize = 256;
const CHANNEL_BATCHES: usize = 64;
/// A partial batch older than this is sent so slow walks stay incremental.
const BATCH_MAX_AGE: Duration = Duration::from_millis(50);

#[derive(Debug)]
pub(crate) enum ScanEvent {
    Found(Vec<(PathBuf, SourceRevision)>),
    Error,
    Done,
}

/// What one bounded [`drain`] took from the scanner.
#[derive(Debug, Default, Eq, PartialEq)]
pub(crate) struct Drained {
    pub(crate) found: usize,
    pub(crate) errors: usize,
    pub(crate) done: bool,
}

/// Takes scan events until the channel is empty, the scan ends, or `budget`
/// has elapsed, so a fast scan cannot stall a frame. At least one available
/// event is always taken.
pub(crate) fn drain(
    receiver: &mpsc::Receiver<ScanEvent>,
    budget: Duration,
    mut found: impl FnMut(PathBuf, SourceRevision),
) -> Drained {
    let started = Instant::now();
    let mut drained = Drained::default();
    loop {
        match receiver.try_recv() {
            Ok(ScanEvent::Found(batch)) => {
                drained.found += batch.len();
                for (path, revision) in batch {
                    found(path, revision);
                }
            }
            Ok(ScanEvent::Error) => drained.errors += 1,
            Ok(ScanEvent::Done) | Err(mpsc::TryRecvError::Disconnected) => {
                drained.done = true;
                break;
            }
            Err(mpsc::TryRecvError::Empty) => break,
        }
        if started.elapsed() >= budget {
            break;
        }
    }
    drained
}

/// Collects found images of one scanner thread into bounded batches. Dropping
/// it sends the remainder, so every path precedes the scan's `Done`.
struct Batcher {
    sender: mpsc::SyncSender<ScanEvent>,
    found: Vec<(PathBuf, SourceRevision)>,
    oldest: Instant,
}

impl Batcher {
    fn new(sender: mpsc::SyncSender<ScanEvent>) -> Self {
        Self {
            sender,
            found: Vec::new(),
            oldest: Instant::now(),
        }
    }

    /// Returns `false` once the receiver is gone.
    fn push(&mut self, path: PathBuf, revision: SourceRevision) -> bool {
        if self.found.is_empty() {
            self.oldest = Instant::now();
        }
        self.found.push((path, revision));
        self.found.len() < BATCH_ENTRIES || self.flush()
    }

    fn error(&mut self) -> bool {
        self.sender.send(ScanEvent::Error).is_ok()
    }

    /// Sends a partial batch that has waited long enough.
    fn flush_if_due(&mut self) -> bool {
        self.found.is_empty() || self.oldest.elapsed() < BATCH_MAX_AGE || self.flush()
    }

    fn flush(&mut self) -> bool {
        self.found.is_empty()
            || self
                .sender
                .send(ScanEvent::Found(std::mem::take(&mut self.found)))
                .is_ok()
    }
}

impl Drop for Batcher {
    fn drop(&mut self) {
        self.flush();
    }
}

pub(crate) fn start(root: PathBuf) -> mpsc::Receiver<ScanEvent> {
    let (sender, receiver) = mpsc::sync_channel(CHANNEL_BATCHES);

    thread::Builder::new()
        .name("red-table-scanner".into())
        .spawn(move || {
            let walker = WalkBuilder::new(root)
                .hidden(false)
                .git_ignore(false)
                .git_global(false)
                .git_exclude(false)
                .follow_links(false)
                .build_parallel();

            walker.run(|| {
                let mut batcher = Batcher::new(sender.clone());
                Box::new(move |result| {
                    let connected = match result {
                        Ok(entry) if entry.file_type().is_some_and(|kind| kind.is_file()) => {
                            let path = entry.into_path();
                            if !is_supported_image(&path) {
                                true
                            } else {
                                match revision_for(&path) {
                                    Ok(revision) => batcher.push(path, revision),
                                    Err(()) => batcher.error(),
                                }
                            }
                        }
                        Ok(_) => true,
                        Err(_) => batcher.error(),
                    };

                    if connected && batcher.flush_if_due() {
                        WalkState::Continue
                    } else {
                        WalkState::Quit
                    }
                })
            });

            let _ = sender.send(ScanEvent::Done);
        })
        .expect("scanner thread should start");

    receiver
}

pub(crate) fn start_paths(root: PathBuf, paths: Vec<PathBuf>) -> mpsc::Receiver<ScanEvent> {
    let (sender, receiver) = mpsc::sync_channel(CHANNEL_BATCHES);
    thread::Builder::new()
        .name("red-table-file-list".into())
        .spawn(move || {
            let mut seen = HashSet::new();
            let mut batcher = Batcher::new(sender.clone());
            for path in paths {
                let path = if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                };
                let connected = match path.canonicalize() {
                    Ok(path) if !seen.insert(path.clone()) => continue,
                    Ok(path) if !is_supported_image(&path) => batcher.error(),
                    Ok(path) => match revision_for(&path) {
                        Ok(revision) => batcher.push(path, revision),
                        Err(()) => batcher.error(),
                    },
                    Err(_) => batcher.error(),
                };
                if !(connected && batcher.flush_if_due()) {
                    return;
                }
            }
            drop(batcher);
            let _ = sender.send(ScanEvent::Done);
        })
        .expect("file-list scanner thread should start");
    receiver
}

pub(crate) fn revision_for(path: &std::path::Path) -> Result<SourceRevision, ()> {
    let metadata = path.metadata().map_err(|_| ())?;
    let modified = metadata.modified().map_err(|_| ())?;
    let modified_nanoseconds = match modified.duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos() as i128,
        Err(error) => -(error.duration().as_nanos() as i128),
    };
    Ok(SourceRevision {
        size: metadata.len(),
        modified_nanoseconds,
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::*;

    fn image(index: usize) -> (PathBuf, SourceRevision) {
        (
            format!("/photos/{index}.png").into(),
            SourceRevision::default(),
        )
    }

    #[test]
    fn batches_flush_on_size_age_and_drop_and_stop_without_a_receiver() {
        let (sender, receiver) = mpsc::sync_channel(8);
        let mut batcher = Batcher::new(sender);
        for index in 0..BATCH_ENTRIES - 1 {
            let (path, revision) = image(index);
            assert!(batcher.push(path, revision));
        }
        assert!(receiver.try_recv().is_err(), "a partial batch waits");
        let (path, revision) = image(BATCH_ENTRIES);
        assert!(batcher.push(path, revision));
        let Ok(ScanEvent::Found(full)) = receiver.try_recv() else {
            panic!("a full batch is sent at once");
        };
        assert_eq!(full.len(), BATCH_ENTRIES);

        let (path, revision) = image(0);
        assert!(batcher.push(path, revision));
        assert!(batcher.flush_if_due());
        assert!(receiver.try_recv().is_err(), "a fresh partial batch waits");
        batcher.oldest = Instant::now() - BATCH_MAX_AGE;
        assert!(batcher.flush_if_due());
        assert!(matches!(receiver.try_recv(), Ok(ScanEvent::Found(batch)) if batch.len() == 1));

        let (path, revision) = image(1);
        assert!(batcher.push(path, revision));
        drop(batcher);
        assert!(
            matches!(receiver.try_recv(), Ok(ScanEvent::Found(batch)) if batch.len() == 1),
            "dropping the batcher delivers the remainder"
        );

        let (sender, receiver) = mpsc::sync_channel(8);
        let mut batcher = Batcher::new(sender);
        drop(receiver);
        assert!(!batcher.error());
        let (path, revision) = image(2);
        batcher.oldest = Instant::now() - BATCH_MAX_AGE;
        batcher.found.push((path, revision));
        assert!(!batcher.flush_if_due(), "a closed receiver stops the walk");
    }

    #[test]
    fn drain_respects_its_budget_but_always_makes_progress() {
        let (sender, receiver) = mpsc::sync_channel(8);
        for _ in 0..3 {
            sender
                .send(ScanEvent::Found(vec![image(0), image(1)]))
                .unwrap();
        }
        sender.send(ScanEvent::Error).unwrap();
        let mut paths = Vec::new();
        let first = drain(&receiver, Duration::ZERO, |path, _| paths.push(path));
        assert_eq!(
            first,
            Drained {
                found: 2,
                errors: 0,
                done: false
            },
            "an exhausted budget still takes one event"
        );
        let rest = drain(&receiver, Duration::from_secs(60), |path, _| {
            paths.push(path)
        });
        assert_eq!((rest.found, rest.errors, rest.done), (4, 1, false));
        assert_eq!(paths.len(), 6);

        sender.send(ScanEvent::Done).unwrap();
        assert!(drain(&receiver, Duration::from_secs(60), |_, _| {}).done);
        drop(sender);
        assert!(
            drain(&receiver, Duration::from_secs(60), |_, _| {}).done,
            "a vanished scanner ends the scan"
        );
    }

    #[test]
    fn large_scans_deliver_every_path_before_done() {
        let root = std::env::temp_dir().join(format!(
            "red-table-scan-large-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let files = BATCH_ENTRIES * 3 + 17;
        for index in 0..files {
            let directory = root.join(format!("album-{}", index % 7));
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join(format!("{index}.jpg")), b"x").unwrap();
        }
        let mut found = HashSet::new();
        let mut events = start(root.clone()).into_iter();
        for event in &mut events {
            match event {
                ScanEvent::Found(batch) => {
                    assert!(batch.len() <= BATCH_ENTRIES);
                    found.extend(batch.into_iter().map(|(path, _)| path));
                }
                ScanEvent::Error => panic!("temporary directory should be readable"),
                ScanEvent::Done => break,
            }
        }
        assert_eq!(found.len(), files);
        assert!(events.next().is_none(), "nothing follows Done");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scanner_reports_supported_files_incrementally() {
        let root = std::env::temp_dir().join(format!(
            "red-table-scan-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("nested")).unwrap();
        fs::write(root.join("one.JPG"), b"content is decoded later").unwrap();
        fs::write(root.join("nested/two.png"), b"content is decoded later").unwrap();
        fs::write(root.join("notes.txt"), b"not an image").unwrap();

        let receiver = start(root.clone());
        let mut found = Vec::new();
        loop {
            match receiver.recv_timeout(Duration::from_secs(2)).unwrap() {
                ScanEvent::Found(batch) => {
                    for (path, revision) in batch {
                        assert!(revision.size > 0);
                        found.push(path);
                    }
                }
                ScanEvent::Error => panic!("temporary directory should be readable"),
                ScanEvent::Done => break,
            }
        }

        assert_eq!(found.len(), 2);
        assert!(found.iter().all(|path| is_supported_image(path)));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn file_list_preserves_order_deduplicates_and_reports_bad_records() {
        let root = std::env::temp_dir().join(format!(
            "red-table-list-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        fs::write(root.join("one.png"), b"decoded later").unwrap();
        fs::write(root.join("notes.txt"), b"not supported").unwrap();
        let receiver = start_paths(
            root.clone(),
            vec![
                "one.png".into(),
                "one.png".into(),
                "missing.jpg".into(),
                "notes.txt".into(),
            ],
        );
        let events = receiver.iter().collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .map(|event| match event {
                    ScanEvent::Found(batch) => batch.len(),
                    _ => 0,
                })
                .sum::<usize>(),
            1
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, ScanEvent::Error))
                .count(),
            2
        );
        assert!(matches!(events.last(), Some(ScanEvent::Done)));
        fs::remove_dir_all(root).unwrap();
    }
}
