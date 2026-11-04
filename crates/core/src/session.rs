//! `RepoSession`: the per-repository actor.
//!
//! - Owns the authoritative [`RepoSnapshot`] and publishes it on a `watch` channel.
//! - Serialises write operations (no `index.lock` contention).
//! - Reads run concurrently on the blocking pool.
//! - A file watcher plus a debounced refresh loop keeps the snapshot current.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use glide_git_backend::{GitCli, Repo, RepoSnapshot};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{watch, Mutex, Notify};

use crate::error::Result;

/// Quiet period used to coalesce bursts of file-system events.
pub const DEBOUNCE: Duration = Duration::from_millis(150);
/// Watcher events arriving this soon after our own write are ignored
/// (we already refresh explicitly after each write).
const SELF_WRITE_GRACE: Duration = Duration::from_millis(300);

pub type SharedSnapshot = Arc<RepoSnapshot>;

struct RefreshState {
    notify: Notify,
    closed: AtomicBool,
    /// Writes in flight; while > 0 watcher events are dropped.
    writing: AtomicU64,
    last_write_end: StdMutex<Option<Instant>>,
    /// Monotonic sequence used to discard results of refreshes that started earlier.
    started_seq: AtomicU64,
    published_seq: StdMutex<u64>,
}

impl RefreshState {
    fn suppressed(&self) -> bool {
        if self.writing.load(Ordering::SeqCst) > 0 {
            return true;
        }
        let last = self.last_write_end.lock().unwrap();
        last.is_some_and(|t| t.elapsed() < SELF_WRITE_GRACE)
    }
}

pub struct RepoSession {
    repo: Arc<Repo>,
    write_lock: Mutex<()>,
    /// Serialises network operations (concurrent fetches fight over FETCH_HEAD and ref locks).
    net_lock: Mutex<()>,
    state: Arc<RefreshState>,
    tx: watch::Sender<Option<SharedSnapshot>>,
    _watcher: Option<RecommendedWatcher>,
}

impl RepoSession {
    /// Open a repository, take the first snapshot and start watching it.
    /// Must be called within a tokio runtime.
    pub async fn open(cli: GitCli, path: impl AsRef<Path>) -> Result<Arc<Self>> {
        let path = path.as_ref().to_path_buf();
        let repo = tokio::task::spawn_blocking(move || Repo::open(cli, path)).await??;
        Self::from_repo(repo).await
    }

    /// Start a session for an already-opened [`Repo`].
    pub async fn from_repo(repo: Repo) -> Result<Arc<Self>> {
        let repo = Arc::new(repo);

        let state = Arc::new(RefreshState {
            notify: Notify::new(),
            closed: AtomicBool::new(false),
            writing: AtomicU64::new(0),
            last_write_end: StdMutex::new(None),
            started_seq: AtomicU64::new(0),
            published_seq: StdMutex::new(0),
        });
        let (tx, _) = watch::channel(None);

        let watcher = match start_watcher(repo.root(), repo.git_dir(), state.clone()) {
            Ok(w) => Some(w),
            Err(e) => {
                tracing::warn!("file watcher unavailable, auto-refresh disabled: {e}");
                None
            }
        };

        let session = Arc::new(Self {
            repo,
            write_lock: Mutex::new(()),
            net_lock: Mutex::new(()),
            state,
            tx,
            _watcher: watcher,
        });
        session.refresh_now().await?;
        tokio::spawn(refresh_loop(
            Arc::downgrade(&session),
            session.state.clone(),
        ));
        Ok(session)
    }

    pub fn root(&self) -> &Path {
        self.repo.root()
    }

    pub fn repo(&self) -> &Repo {
        &self.repo
    }

    /// Latest published snapshot.
    pub fn snapshot(&self) -> Option<SharedSnapshot> {
        self.tx.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Option<SharedSnapshot>> {
        self.tx.subscribe()
    }

    /// Ask for a debounced background refresh.
    pub fn request_refresh(&self) {
        self.state.notify.notify_one();
    }

    /// Recompute the snapshot immediately and publish it.
    pub async fn refresh_now(&self) -> Result<SharedSnapshot> {
        let seq = self.state.started_seq.fetch_add(1, Ordering::SeqCst) + 1;
        let repo = self.repo.clone();
        let snap = Arc::new(tokio::task::spawn_blocking(move || repo.snapshot()).await??);
        self.publish(seq, snap.clone());
        Ok(snap)
    }

    fn publish(&self, seq: u64, snap: SharedSnapshot) {
        let mut published = self.state.published_seq.lock().unwrap();
        if seq < *published {
            // A refresh that started later already published fresher data.
            return;
        }
        *published = seq;
        self.tx.send_if_modified(|cur| {
            if cur.as_deref() == Some(&*snap) {
                false
            } else {
                *cur = Some(snap);
                true
            }
        });
    }

    /// Run a read-only operation on the blocking pool (concurrent with other reads).
    pub async fn read<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Repo) -> glide_git_backend::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let repo = self.repo.clone();
        Ok(tokio::task::spawn_blocking(move || f(&repo)).await??)
    }

    /// Run a mutating operation. Writes are serialised; the snapshot is
    /// refreshed afterwards whether the operation succeeded or not.
    pub async fn write<T, F>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Repo) -> glide_git_backend::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let _guard = self.write_lock.lock().await;
        self.state.writing.fetch_add(1, Ordering::SeqCst);
        let repo = self.repo.clone();
        let result = tokio::task::spawn_blocking(move || f(&repo)).await;
        *self.state.last_write_end.lock().unwrap() = Some(Instant::now());
        self.state.writing.fetch_sub(1, Ordering::SeqCst);
        drop(_guard);

        if let Err(e) = self.refresh_now().await {
            tracing::warn!("refresh after write failed: {e}");
        }
        Ok(result??)
    }
}

impl RepoSession {
    /// Run a network operation. Remote ops are serialised among themselves.
    ///
    /// `touches_worktree = false` (fetch, push) only updates refs, so it runs
    /// alongside local writes — staging and committing stay responsive during a
    /// slow push. `true` (pull) also takes the write lock.
    pub async fn remote<T, F>(&self, touches_worktree: bool, f: F) -> Result<T>
    where
        F: FnOnce(&Repo) -> glide_git_backend::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let _net = self.net_lock.lock().await;
        if touches_worktree {
            return self.write(f).await;
        }
        let repo = self.repo.clone();
        let result = tokio::task::spawn_blocking(move || f(&repo)).await;
        if let Err(e) = self.refresh_now().await {
            tracing::warn!("refresh after remote operation failed: {e}");
        }
        Ok(result??)
    }
}

impl Drop for RepoSession {
    fn drop(&mut self) {
        self.state.closed.store(true, Ordering::SeqCst);
        self.state.notify.notify_one();
    }
}

async fn refresh_loop(session: std::sync::Weak<RepoSession>, state: Arc<RefreshState>) {
    loop {
        state.notify.notified().await;
        if state.closed.load(Ordering::SeqCst) {
            return;
        }
        // Debounce: keep sleeping while new notifications keep arriving.
        loop {
            tokio::select! {
                _ = tokio::time::sleep(DEBOUNCE) => break,
                _ = state.notify.notified() => {
                    if state.closed.load(Ordering::SeqCst) { return; }
                }
            }
        }
        let Some(s) = session.upgrade() else { return };
        if let Err(e) = s.refresh_now().await {
            tracing::warn!("background refresh failed: {e}");
        }
    }
}

fn start_watcher(
    root: &Path,
    git_dir: &Path,
    state: Arc<RefreshState>,
) -> notify::Result<RecommendedWatcher> {
    let git_dir_owned: PathBuf = git_dir.to_path_buf();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        if state.suppressed() {
            return;
        }
        let relevant = event.paths.iter().any(|p| is_relevant(p, &git_dir_owned));
        if relevant {
            state.notify.notify_one();
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    // Linked worktrees / separate git dirs live outside the worktree.
    if !git_dir.starts_with(root) {
        watcher.watch(git_dir, RecursiveMode::Recursive)?;
    }
    Ok(watcher)
}

/// Worktree changes are always relevant; inside the git dir only the files
/// that affect status, refs or in-progress operations are.
pub fn is_relevant(path: &Path, git_dir: &Path) -> bool {
    // FSEvents may report canonical paths (/private/var vs /var); compare loosely.
    let in_git = path.starts_with(git_dir) || path.components().any(|c| c.as_os_str() == ".git");
    if !in_git {
        return true;
    }
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.ends_with(".lock") {
        return false;
    }
    let s = path.to_string_lossy().replace('\\', "/");
    const INTERESTING: &[&str] = &[
        "/HEAD",
        "/index",
        "/packed-refs",
        "/MERGE_HEAD",
        "/CHERRY_PICK_HEAD",
        "/REVERT_HEAD",
        "/FETCH_HEAD",
    ];
    if INTERESTING.iter().any(|f| s.ends_with(f)) {
        return true;
    }
    s.contains("/refs/") || s.contains("/rebase-merge") || s.contains("/rebase-apply")
}

#[cfg(test)]
mod tests {
    use super::is_relevant;
    use std::path::Path;

    #[test]
    fn relevance_filter() {
        let git = Path::new("/r/.git");
        assert!(is_relevant(Path::new("/r/src/main.rs"), git));
        assert!(is_relevant(Path::new("/r/.git/index"), git));
        assert!(is_relevant(Path::new("/r/.git/HEAD"), git));
        assert!(is_relevant(Path::new("/r/.git/refs/heads/main"), git));
        assert!(is_relevant(Path::new("/r/.git/rebase-merge/done"), git));
        assert!(!is_relevant(Path::new("/r/.git/index.lock"), git));
        assert!(!is_relevant(Path::new("/r/.git/objects/ab/cdef"), git));
        assert!(!is_relevant(Path::new("/r/.git/logs/HEAD.tmp"), git));
    }
}
