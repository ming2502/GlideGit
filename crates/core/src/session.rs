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

use glide_git_backend::{tr, CancelToken, GitCli, GitError, Repo, RepoSnapshot};
use glide_log_index::{
    AuthorCount, IndexError, LogFilter, LogIndex, LogPage, LogStatus, UpdateKind,
};
use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::{watch, Mutex, Notify};

use crate::cache::{fnv1a, SnapshotCache};
use crate::error::{CoreError, Result};
use crate::oplog::{self, Context, OpEntry, OpKind, OpLog, OpMeta, Saved};

/// Quiet period used to coalesce bursts of file-system events.
pub const DEBOUNCE: Duration = Duration::from_millis(150);
/// Wait before refreshing after the first change following a quiet period.
/// Short, so a saved file shows up at once; editors that save in several
/// steps (temp file, rename) finish well within it. If more changes keep
/// coming (a checkout, a build), the full [`DEBOUNCE`] applies instead.
pub const FIRST_DELAY: Duration = Duration::from_millis(40);
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
    /// Last snapshot on disk, for an instant start next time.
    cache: Option<Arc<SnapshotCache>>,
    /// Recorded operations, for undo.
    oplog: Arc<StdMutex<OpLog>>,
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
    /// Refs fingerprint at the last successful update, and when.
    last_refs: StdMutex<Option<(u64, Instant)>>,
}

/// Update the commit index at least this often even when refs look unchanged.
const LOG_MAX_AGE: Duration = Duration::from_secs(60);

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

    /// Start a session; with `cache_dir`, also keep the commit index for the
    /// Log view and the last snapshot there.
    ///
    /// With a cached snapshot the session starts at once with it (marked
    /// stale) and refreshes in the background; otherwise the first snapshot
    /// is taken before returning.
    pub async fn from_repo_with(repo: Repo, cache_dir: Option<PathBuf>) -> Result<Arc<Self>> {
        let repo = Arc::new(repo);
        let cache = cache_dir
            .as_deref()
            .map(|d| Arc::new(SnapshotCache::new(&d.join("snapshots"), repo.root())));
        let cached = match &cache {
            Some(c) => {
                let c = c.clone();
                tokio::task::spawn_blocking(move || c.load()).await?
            }
            None => None,
        };
        let oplog_file = cache_dir.as_deref().map(|d| {
            let root = repo.root().to_string_lossy();
            d.join("oplog")
                .join(format!("{:016x}.jsonl", fnv1a(root.as_bytes())))
        });
        let oplog = Arc::new(StdMutex::new(
            tokio::task::spawn_blocking(move || OpLog::open(oplog_file)).await?,
        ));
        let log = match cache_dir.map(|d| d.join("log-index")) {
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
                            last_refs: StdMutex::new(None),
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
            cache,
            oplog,
            _watcher: watcher,
        });
        match cached {
            Some(snap) => {
                session.tx.send_replace(Some(Arc::new(snap)));
                let s = session.clone();
                tokio::spawn(async move {
                    if let Err(e) = s.refresh_now().await {
                        tracing::warn!("first refresh failed: {e}");
                    }
                });
            }
            None => {
                session.refresh_now().await?;
            }
        }
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
        let changed = self.tx.send_if_modified(|cur| {
            if cur.as_deref() == Some(&*snap) {
                false
            } else {
                *cur = Some(snap.clone());
                true
            }
        });
        if changed {
            if let Some(c) = &self.cache {
                c.store(snap);
            }
        }
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
    /// Run a mutating operation and record it in the operation log, with how
    /// to undo it (see [`crate::oplog`]).
    pub async fn write_op<T, F>(&self, meta: OpMeta, f: F) -> Result<T>
    where
        F: FnOnce(&Repo) -> glide_git_backend::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let log = self.oplog.clone();
        self.write(move |repo| record(repo, &log, meta, f)).await
    }

    /// [`RepoSession::remote`], recorded in the operation log.
    pub async fn remote_op<T, F>(&self, touches_worktree: bool, meta: OpMeta, f: F) -> Result<T>
    where
        F: FnOnce(&Repo) -> glide_git_backend::Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let log = self.oplog.clone();
        self.remote(touches_worktree, move |repo| record(repo, &log, meta, f))
            .await
    }

    /// Recorded operations, newest first.
    pub fn operations(&self) -> Vec<OpEntry> {
        self.oplog.lock().unwrap().entries()
    }

    /// Undo a recorded operation, if the repository is still where it left
    /// things. Returns what was done.
    pub async fn undo(&self, id: u64) -> Result<String> {
        let log = self.oplog.clone();
        self.write(move |repo| {
            let entry = log.lock().unwrap().get(id).cloned().ok_or_else(|| {
                GitError::Invalid(tr!("没有这条操作记录", "No such operation in the log"))
            })?;
            if entry.undone {
                return Err(GitError::Invalid(tr!(
                    "这个操作已经撤销过了",
                    "This operation was already undone"
                )));
            }
            if entry.steps.is_empty() {
                return Err(GitError::Invalid(entry.blocked.unwrap_or_else(|| {
                    tr!("这个操作无法撤销", "This operation cannot be undone")
                })));
            }
            oplog::check(repo, &entry.steps).map_err(GitError::Invalid)?;
            let what = entry.undo.clone().unwrap_or_default();
            let meta = OpMeta::new(OpKind::Undo, tr!("撤销：{}", "Undo: {}", entry.title));
            record(repo, &log, meta, |r| oplog::run(r, &entry.steps))?;
            log.lock().unwrap().mark_undone(id);
            Ok(what)
        })
        .await
    }
}

/// Run `f`, comparing the refs before and after, and log it.
fn record<T>(
    repo: &Repo,
    log: &StdMutex<OpLog>,
    meta: OpMeta,
    f: impl FnOnce(&Repo) -> glide_git_backend::Result<T>,
) -> glide_git_backend::Result<T> {
    let before = repo.refs_state();
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let mut saved = match &meta.backup {
        Some(b) => {
            let id = log.lock().unwrap().next_id();
            let dir = repo
                .git_dir()
                .join("glidegit-backup")
                .join(format!("{time}-{id}"));
            oplog::take_backup(repo, b, &dir)?
        }
        None => Saved::default(),
    };
    let result = f(repo);
    let (Ok(before), Ok(after)) = (before, repo.refs_state()) else {
        return result;
    };
    oplog::seal_backup(repo, &mut saved);
    let ctx = Context {
        in_progress: repo.operation().is_some(),
        orig_head: std::fs::read_to_string(repo.git_dir().join("ORIG_HEAD"))
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| s.len() >= 40),
    };
    let changed = before != after || ctx.in_progress || !saved.files.is_empty();
    if result.is_err() && !changed {
        return result;
    }
    let (steps, undo, blocked) = match oplog::plan(&meta, &before, &after, &saved, &ctx) {
        Ok((steps, what)) => (steps, Some(what), None),
        // Nothing happened worth listing (checking out the current branch...).
        Err(_) if !changed && meta.kind != OpKind::Push => return result,
        Err(why) => (Vec::new(), None, Some(why)),
    };
    let title = match &result {
        Ok(_) => meta.title,
        Err(_) => tr!("{}（未完成）", "{} (not finished)", meta.title),
    };
    log.lock().unwrap().push(OpEntry {
        id: 0,
        time,
        kind: meta.kind,
        title,
        undo,
        blocked,
        undone: false,
        steps,
    });
    result
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
        let r = repo.clone();
        let result = tokio::task::spawn_blocking(move || {
            // Most refreshes come from file edits; when no ref moved there is
            // nothing to do (and no git process to start).
            let fingerprint = r.refs_fingerprint();
            let fresh = l
                .last_refs
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some_and(|(fp, at)| fp == fingerprint && at.elapsed() < LOG_MAX_AGE);
            if fresh {
                return Ok::<_, IndexError>(None);
            }
            let kind = l.index.update(&r, &l.cancel, &|status| {
                l.tx.send_if_modified(|cur| {
                    if *cur == status {
                        false
                    } else {
                        *cur = status;
                        true
                    }
                });
            })?;
            *l.last_refs.lock().unwrap_or_else(|e| e.into_inner()) =
                Some((fingerprint, Instant::now()));
            Ok(Some(kind))
        })
        .await;
        match result {
            Ok(Ok(Some(UpdateKind::Full))) => {
                // First build done: add a commit-graph if the repo has none,
                // so the next full walk and path filters are fast.
                let repo = repo.clone();
                tokio::task::spawn_blocking(move || match repo.ensure_commit_graph() {
                    Ok(true) => tracing::info!("wrote commit-graph for {}", repo.root().display()),
                    Ok(false) => {}
                    Err(e) => tracing::info!("commit-graph not written: {e}"),
                });
            }
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
        // A short window first, which takes in the several events one save
        // makes (they come within microseconds). Changes still arriving late
        // in it mean something bigger is going on (a checkout, a build):
        // then wait until things have been quiet for the full debounce.
        let start = Instant::now();
        let mut last = start;
        let window = tokio::time::sleep(FIRST_DELAY);
        tokio::pin!(window);
        loop {
            tokio::select! {
                _ = &mut window => break,
                _ = state.notify.notified() => {
                    if state.closed.load(Ordering::SeqCst) { return; }
                    last = Instant::now();
                }
            }
        }
        if last.duration_since(start) > FIRST_DELAY / 2 {
            loop {
                tokio::select! {
                    _ = tokio::time::sleep(DEBOUNCE) => break,
                    _ = state.notify.notified() => {
                        if state.closed.load(Ordering::SeqCst) { return; }
                    }
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
