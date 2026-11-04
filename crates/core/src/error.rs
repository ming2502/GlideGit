pub type Result<T, E = CoreError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum CoreError {
    #[error(transparent)]
    Git(#[from] glide_git_backend::GitError),

    #[error("background task failed: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("repository is not open: {0}")]
    NotOpen(String),
}

impl CoreError {
    pub fn kind(&self) -> &'static str {
        match self {
            CoreError::Git(e) => e.kind(),
            CoreError::Join(_) => "internal",
            CoreError::NotOpen(_) => "notOpen",
        }
    }
}
