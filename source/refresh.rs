use std::{
    path::Path,
    sync::{Arc, mpsc},
    thread,
};

use crate::{
    app::{EntryId, SourceRevision},
    scan::revision_for,
};

#[derive(Clone, Debug)]
pub(crate) struct RevisionCheck {
    pub(crate) id: EntryId,
    pub(crate) path: Arc<Path>,
    pub(crate) previous: SourceRevision,
}

#[derive(Debug)]
pub(crate) struct RevisionChange {
    pub(crate) id: EntryId,
    pub(crate) previous: SourceRevision,
    pub(crate) current: SourceRevision,
}

/// One outstanding working-set batch; metadata I/O never runs on the UI thread.
pub(crate) struct RevisionMonitor {
    requests: mpsc::SyncSender<Vec<RevisionCheck>>,
    responses: mpsc::Receiver<Vec<RevisionChange>>,
    pending: bool,
}

impl RevisionMonitor {
    pub(crate) fn new() -> Self {
        let (requests, receiver) = mpsc::sync_channel::<Vec<RevisionCheck>>(1);
        let (sender, responses) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("red-table-revisions".into())
            .spawn(move || {
                while let Ok(checks) = receiver.recv() {
                    if sender.send(check_revisions(checks)).is_err() {
                        break;
                    }
                }
            })
            .expect("revision worker should start");
        Self {
            requests,
            responses,
            pending: false,
        }
    }

    pub(crate) fn is_pending(&self) -> bool {
        self.pending
    }

    pub(crate) fn request(&mut self, checks: Vec<RevisionCheck>) -> bool {
        if self.pending || checks.is_empty() {
            return false;
        }
        self.pending = self.requests.try_send(checks).is_ok();
        self.pending
    }

    pub(crate) fn drain(&mut self) -> Vec<RevisionChange> {
        match self.responses.try_recv() {
            Ok(changes) => {
                self.pending = false;
                changes
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.pending = false;
                Vec::new()
            }
            Err(mpsc::TryRecvError::Empty) => Vec::new(),
        }
    }
}

fn check_revisions(checks: Vec<RevisionCheck>) -> Vec<RevisionChange> {
    checks
        .into_iter()
        .filter_map(|check| {
            let current = revision_for(&check.path).ok()?;
            (current != check.previous).then_some(RevisionChange {
                id: check.id,
                previous: check.previous,
                current,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        fs,
        time::{Duration, SystemTime},
    };

    #[test]
    fn monitors_overwrites_atomic_replacements_and_missing_file_recovery() {
        let root = std::env::temp_dir().join(format!(
            "red-table-refresh-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(SystemTime::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        let path = root.join("image.png");
        fs::write(&path, b"old").unwrap();
        let mut check = RevisionCheck {
            id: EntryId(42),
            path: Arc::from(path.as_path()),
            previous: revision_for(&path).unwrap(),
        };
        let mut monitor = RevisionMonitor::new();
        let poll = |monitor: &mut RevisionMonitor, check: &RevisionCheck| {
            assert!(monitor.request(vec![check.clone()]));
            assert!(
                !monitor.request(vec![check.clone()]),
                "only one batch may be outstanding"
            );
            let changes = monitor
                .responses
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
            monitor.pending = false;
            changes
        };
        assert!(poll(&mut monitor, &check).is_empty());
        // Same length with a deliberate mtime change must also invalidate caches.
        fs::write(&path, b"new").unwrap();
        fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::UNIX_EPOCH + Duration::from_secs(42))
            .unwrap();
        let changes = poll(&mut monitor, &check);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].id, check.id);
        assert_eq!(changes[0].previous, check.previous);
        check.previous = changes[0].current;

        let replacement = root.join("replacement.png");
        fs::write(&replacement, b"atomic replacement").unwrap();
        fs::rename(replacement, &path).unwrap();
        let changes = poll(&mut monitor, &check);
        assert_eq!(changes.len(), 1);
        check.previous = changes[0].current;
        fs::remove_file(&path).unwrap();
        assert!(poll(&mut monitor, &check).is_empty());
        fs::write(&path, b"restored").unwrap();
        assert_eq!(poll(&mut monitor, &check).len(), 1);
        fs::remove_dir_all(root).unwrap();
    }
}
