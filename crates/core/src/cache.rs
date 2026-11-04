//! The last snapshot of each repository, kept on disk so a repository opens
//! instantly (stale-while-revalidate): the UI shows the cached state at once,
//! marked stale, while the real `git status` runs in the background.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use glide_git_backend::RepoSnapshot;
use serde::{Deserialize, Serialize};

/// Bump when the stored layout changes; older files are ignored.
const FORMAT: u32 = 1;

#[derive(Serialize, Deserialize)]
struct Stored {
    format: u32,
    snapshot: RepoSnapshot,
}

/// Stable across runs and Rust versions (unlike `DefaultHasher`).
pub(crate) fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

pub(crate) struct SnapshotCache {
    path: PathBuf,
    root: String,
    /// Newest snapshot not yet written; writes are coalesced.
    pending: Mutex<Option<Arc<RepoSnapshot>>>,
    writing: AtomicBool,
}

impl SnapshotCache {
    pub fn new(dir: &Path, root: &Path) -> Self {
        let root = root.to_string_lossy().into_owned();
        Self {
            path: dir.join(format!("{:016x}.json", fnv1a(root.as_bytes()))),
            root,
            pending: Mutex::new(None),
            writing: AtomicBool::new(false),
        }
    }

    /// The cached snapshot, marked stale; `None` if missing or unusable.
    pub fn load(&self) -> Option<RepoSnapshot> {
        let bytes = std::fs::read(&self.path).ok()?;
        let stored: Stored = serde_json::from_slice(&bytes).ok()?;
        if stored.format != FORMAT || stored.snapshot.root != self.root {
            return None;
        }
        let mut snap = stored.snapshot;
        snap.stale = true;
        Some(snap)
    }

    /// Remember `snap`; it is written on a blocking thread, and when several
    /// arrive while a write is running only the newest is written next.
    pub fn store(self: &Arc<Self>, snap: Arc<RepoSnapshot>) {
        if snap.stale {
            return;
        }
        *self.pending.lock().unwrap_or_else(|e| e.into_inner()) = Some(snap);
        if self.writing.swap(true, Ordering::SeqCst) {
            return; // the running writer picks it up
        }
        let me = self.clone();
        tokio::task::spawn_blocking(move || loop {
            let next = me.pending.lock().unwrap_or_else(|e| e.into_inner()).take();
            match next {
                Some(s) => {
                    if let Err(e) = me.write(&s) {
                        tracing::debug!("snapshot cache not written: {e}");
                    }
                }
                None => {
                    me.writing.store(false, Ordering::SeqCst);
                    // A store() may have slipped in after take() returned None.
                    let more = me
                        .pending
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_some();
                    if !more || me.writing.swap(true, Ordering::SeqCst) {
                        return;
                    }
                }
            }
        });
    }

    fn write(&self, snap: &RepoSnapshot) -> std::io::Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let data = serde_json::to_vec(&Stored {
            format: FORMAT,
            snapshot: snap.clone(),
        })
        .map_err(std::io::Error::other)?;
        // Write then rename, so a crash never leaves a half-written file.
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, data)?;
        std::fs::rename(&tmp, &self.path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glide_git_backend::{BranchList, HeadInfo};

    fn snap(root: &str) -> RepoSnapshot {
        RepoSnapshot {
            root: root.into(),
            head: HeadInfo::default(),
            operation: None,
            changes: Vec::new(),
            branches: BranchList::default(),
            stale: false,
            operation_info: None,
        }
    }

    #[tokio::test]
    async fn round_trip_marks_stale_and_checks_root() {
        let dir = tempfile::tempdir().unwrap();
        let c = Arc::new(SnapshotCache::new(dir.path(), Path::new("/repo")));
        assert!(c.load().is_none());
        c.store(Arc::new(snap("/repo")));
        for _ in 0..100 {
            if c.load().is_some() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let loaded = c.load().expect("written");
        assert!(loaded.stale);
        // A file for another root (hash collision, moved repo) is ignored.
        std::fs::copy(
            &c.path,
            SnapshotCache::new(dir.path(), Path::new("/other")).path,
        )
        .unwrap();
        assert!(SnapshotCache::new(dir.path(), Path::new("/other"))
            .load()
            .is_none());
        // Stale snapshots are never written back.
        let mut s = snap("/repo");
        s.stale = true;
        s.changes.clear();
        c.store(Arc::new(s));
        // Garbage is ignored, not an error.
        std::fs::write(&c.path, b"{not json").unwrap();
        assert!(c.load().is_none());
    }
}
