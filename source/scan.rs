use std::{collections::HashSet, path::PathBuf, sync::mpsc, thread, time::UNIX_EPOCH};

use ignore::{WalkBuilder, WalkState};

use crate::app::{SourceRevision, is_supported_image};

#[derive(Debug)]
pub(crate) enum ScanEvent {
    Found {
        path: PathBuf,
        revision: SourceRevision,
    },
    Error,
    Done,
}

pub(crate) fn start(root: PathBuf) -> mpsc::Receiver<ScanEvent> {
    let (sender, receiver) = mpsc::sync_channel(1024);

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
                let sender = sender.clone();
                Box::new(move |result| {
                    let event = match result {
                        Ok(entry) if entry.file_type().is_some_and(|kind| kind.is_file()) => {
                            let path = entry.into_path();
                            if !is_supported_image(&path) {
                                None
                            } else {
                                match revision_for(&path) {
                                    Ok(revision) => Some(ScanEvent::Found { path, revision }),
                                    Err(()) => Some(ScanEvent::Error),
                                }
                            }
                        }
                        Ok(_) => None,
                        Err(_) => Some(ScanEvent::Error),
                    };

                    if event.is_some_and(|event| sender.send(event).is_err()) {
                        WalkState::Quit
                    } else {
                        WalkState::Continue
                    }
                })
            });

            let _ = sender.send(ScanEvent::Done);
        })
        .expect("scanner thread should start");

    receiver
}

pub(crate) fn start_paths(root: PathBuf, paths: Vec<PathBuf>) -> mpsc::Receiver<ScanEvent> {
    let (sender, receiver) = mpsc::sync_channel(1024);
    thread::Builder::new()
        .name("red-table-file-list".into())
        .spawn(move || {
            let mut seen = HashSet::new();
            for path in paths {
                let path = if path.is_absolute() {
                    path
                } else {
                    root.join(path)
                };
                let event = match path.canonicalize() {
                    Ok(path) if !seen.insert(path.clone()) => continue,
                    Ok(path) if !is_supported_image(&path) => ScanEvent::Error,
                    Ok(path) => match revision_for(&path) {
                        Ok(revision) => ScanEvent::Found { path, revision },
                        Err(()) => ScanEvent::Error,
                    },
                    Err(_) => ScanEvent::Error,
                };
                if sender.send(event).is_err() {
                    return;
                }
            }
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
    use std::{fs, time::Duration};

    use super::*;

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
                ScanEvent::Found { path, revision } => {
                    assert!(revision.size > 0);
                    found.push(path);
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
                .filter(|event| matches!(event, ScanEvent::Found { .. }))
                .count(),
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
