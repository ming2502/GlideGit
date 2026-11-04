//! Registry of open repositories, keyed by canonical worktree root.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use glide_git_backend::{GitCli, Repo};
use tokio::sync::RwLock;

use crate::error::{CoreError, Result};
use crate::session::RepoSession;

pub struct RepositoryManager {
    cli: GitCli,
    sessions: RwLock<HashMap<String, Arc<RepoSession>>>,
}

impl RepositoryManager {
    pub fn new(cli: GitCli) -> Self {
        Self {
            cli,
            sessions: RwLock::new(HashMap::new()),
        }
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
        let session = RepoSession::from_repo(repo).await?;
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
