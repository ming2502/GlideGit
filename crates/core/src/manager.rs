//! Registry of open repositories, keyed by canonical worktree root.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock as StdRwLock};

use glide_git_backend::{GitCli, Repo};
use tokio::sync::RwLock;

use crate::error::{CoreError, Result};
use crate::session::RepoSession;

pub struct RepositoryManager {
    cli: GitCli,
    sessions: RwLock<HashMap<String, Arc<RepoSession>>>,
    /// Where commit indexes are stored; `None` disables the Log index.
    log_cache: StdRwLock<Option<PathBuf>>,
}

impl RepositoryManager {
    pub fn new(cli: GitCli) -> Self {
        Self {
            cli,
            sessions: RwLock::new(HashMap::new()),
            log_cache: StdRwLock::new(None),
        }
    }

    /// Store commit indexes under `dir` for repositories opened from now on.
    pub fn set_log_cache_dir(&self, dir: PathBuf) {
        *self.log_cache.write().unwrap_or_else(|e| e.into_inner()) = Some(dir);
    }

    pub fn cli(&self) -> &GitCli {
        &self.cli
    }

    /// Open (or return the already-open) session for the repo containing `path`.
    /// Returns `(session, newly_opened)`.
    pub async fn open(&self, path: impl AsRef<Path>) -> Result<(Arc<RepoSession>, bool)> {
        let cli = self.cli.clone();
        let path = path.as_ref().to_path_buf();
        let repo = tokio::task::spawn_blocking(move || Repo::open(cli, path)).await??;
        let key = repo.root().to_string_lossy().into_owned();
        if let Some(existing) = self.sessions.read().await.get(&key) {
            return Ok((existing.clone(), false));
        }
        let log_cache = self
            .log_cache
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let session = RepoSession::from_repo_with(repo, log_cache).await?;
        let mut map = self.sessions.write().await;
        // Another caller may have opened it while we were taking the first snapshot.
        let entry = map.entry(key).or_insert_with(|| session.clone());
        let newly = Arc::ptr_eq(entry, &session);
        Ok((entry.clone(), newly))
    }

    pub async fn get(&self, root: &str) -> Result<Arc<RepoSession>> {
        self.sessions
            .read()
            .await
            .get(root)
            .cloned()
            .ok_or_else(|| CoreError::NotOpen(root.to_string()))
    }

    pub async fn close(&self, root: &str) -> bool {
        self.sessions.write().await.remove(root).is_some()
    }
}
