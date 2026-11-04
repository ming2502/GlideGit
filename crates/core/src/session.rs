//! `RepoSession`: the per-repository actor.
//!
//! - Owns the authoritative [`RepoSnapshot`] and publishes it on a `watch` channel.
//! - Serialises write operations (no `index.lock` contention).
//! - Reads run concurrently on the blocking pool.
//! - A file watcher plus a debounced refresh loop keeps the snapshot current.
//! - A background loop keeps the commit index for the Log view current
//!   (lowest priority: it never holds the write lock).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use glide_git_backend::{CancelToken, GitCli, Repo, RepoSnapshot};
use glide_log_index::{AuthorCount, LogFilter, LogIndex, LogPage, LogStatus};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{watch, Mutex, Notify};

use crate::error::{CoreError, Result};

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
    log: Option<Arc<LogDriver>>,
    _watcher: Option<RecommendedWatcher>,
}

/// Owns the commit index and the loop that updates it.
struct LogDriver {
    index: Arc<LogIndex>,
    notify: Notify,
    cancel: CancelToken,
    tx: watch::Sender<LogStatus>,
    /// The filter the UI asked for last. Asking for a different one cancels
    /// work still running for the previous filter (typing in the search box).
    filter_job: StdMutex<(LogFilter, CancelToken)>,
}

impl RepoSession {
    /// Open a repository, take the first snapshot and start watching it.
    /// Must be called within a tokio runtime.
    pub async fn open(cli: GitCli, path: impl AsRef<Path>) -> Result<Arc<Self>> {
        let path = path.as_ref().to_path_buf();
        let repo = tokio::task::spawn_blocking(move || Repo::open(cli, path)).await??;
        Self::from_repo(repo).await
    }

    /// Start a session for an already-opened [`Repo`], without a commit index.
    pub async fn from_repo(repo: Repo) -> Result<Arc<Self>> {
        Self::from_repo_with(repo, None).await
    }

    /// Start a session; with `log_cache`, also maintain the commit index for
    /// the Log view in that directory.
    pub async fn from_repo_with(repo: Repo, log_cache: Option<PathBuf>) -> Result<Arc<Self>> {
        let repo = Arc::new(repo);
        let log = match log_cache {
            Some(dir) => {
                let root = repo.root().to_path_buf();
                match tokio::task::spawn_blocking(move || LogIndex::open(&dir, &root)).await? {
                    Ok(index) => {
                        let (tx, _) = watch::channel(index.status());
                        Some(Arc::new(LogDriver {
                            index: Arc::new(index),
                            notify: Notify::new(),
                            cancel: CancelToken::new(),
                            tx,
                            filter_job: StdMutex::new((LogFilter::default(), CancelToken::new())),
                        }))
                    }
                    Err(e) => {
                        tracing::warn!("commit index unavailable: {e}");
                        None
                    }
                }
            }
            None => None,
        };

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
            log,
            _watcher: watcher,
        });
        session.refresh_now().await?;
        tokio::spawn(refresh_loop(
            Arc::downgrade(&session),
            session.state.clone(),
        ));
        if let Some(log) = &session.log {
            tokio::spawn(log_loop(log.clone(), session.repo.clone()));
        }
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
        // Refs may have moved; the index update is a no-op when they did not.
        if let Some(log) = &self.log {
            log.notify.notify_one();
        }
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

impl RepoSession {
    /// Current state of the commit index, `None` if this session has none.
    pub fn log_status(&self) -> Option<LogStatus> {
        self.log.as_ref().map(|l| l.tx.borrow().clone())
    }

    pub fn subscribe_log(&self) -> Option<watch::Receiver<LogStatus>> {
        self.log.as_ref().map(|l| l.tx.subscribe())
    }

    fn log_driver(&self) -> Result<&Arc<LogDriver>> {
        self.log.as_ref().ok_or(CoreError::NoLogIndex)
    }

    /// Cancellation token for work on `filter`; a new filter cancels the old one.
    fn filter_token(log: &LogDriver, filter: &LogFilter) -> CancelToken {
        let mut job = log.filter_job.lock().unwrap_or_else(|e| e.into_inner());
        if job.0 != *filter {
            job.1.cancel();
            *job = (filter.clone(), CancelToken::new());
        }
        job.1.clone()
    }

    /// Rows `start..start + count` of the Log, optionally filtered.
    pub async fn log_page(&self, start: u32, count: u32, filter: LogFilter) -> Result<LogPage> {
        let log = self.log_driver()?;
        let index = log.index.clone();
        let cancel = Self::filter_token(log, &filter);
        let repo = self.repo.clone();
        Ok(tokio::task::spawn_blocking(move || {
            index.page_filtered(&repo, &filter, &cancel, start, count)
        })
        .await??)
    }

    /// Row number of a commit in the (filtered) Log.
    pub async fn log_row_of(&self, oid: String, filter: LogFilter) -> Result<Option<u32>> {
        let log = self.log_driver()?;
        let index = log.index.clone();
        let cancel = Self::filter_token(log, &filter);
        let repo = self.repo.clone();
        Ok(tokio::task::spawn_blocking(move || {
            index.row_of_filtered(&repo, &filter, &cancel, &oid)
        })
        .await??)
    }

    /// Authors matching `query`, most commits first (for the author filter).
    pub async fn log_authors(&self, query: String, limit: u32) -> Result<Vec<AuthorCount>> {
        let index = self.log_driver()?.index.clone();
        Ok(tokio::task::spawn_blocking(move || index.authors(&query, limit)).await??)
    }
}

impl Drop for RepoSession {
    fn drop(&mut self) {
        self.state.closed.store(true, Ordering::SeqCst);
        self.state.notify.notify_one();
        if let Some(log) = &self.log {
            log.cancel.cancel();
            log.notify.notify_one();
        }
    }
}

/// Runs index updates one at a time; requests that arrive meanwhile collapse
/// into a single follow-up run.
async fn log_loop(log: Arc<LogDriver>, repo: Arc<Repo>) {
    loop {
        log.notify.notified().await;
        if log.cancel.is_cancelled() {
            return;
        }
        let l = log.clone();
        let repo = repo.clone();
        let result = tokio::task::spawn_blocking(move || {
            l.index.update(&repo, &l.cancel, &|status| {
                l.tx.send_if_modified(|cur| {
                    if *cur == status {
                        false
                    } else {
                        *cur = status;
                        true
                    }
                });
            })
        })
        .await;
        match result {
            Ok(Ok(_)) => {}
            Ok(Err(e)) if e.is_cancelled() => {}
            Ok(Err(e)) => tracing::warn!("commit index update failed: {e}"),
            Err(e) => tracing::warn!("commit index update panicked: {e}"),
        }
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
