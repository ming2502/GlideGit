use std::io;

pub type Result<T, E = GitError> = std::result::Result<T, E>;

#[derive(Debug, thiserror::Error)]
pub enum GitError {
    #[error("git executable not found or not runnable: {0}")]
    GitNotFound(String),

    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// git exited with a non-zero status. `message` is the trimmed stderr.
    #[error("{message}")]
    Failed {
        command: String,
        code: Option<i32>,
        message: String,
    },

    #[error("not a git repository: {0}")]
    NotARepository(String),

    #[error("operation cancelled")]
    Cancelled,

    #[error("operation timed out")]
    TimedOut,

    #[error("unexpected git output: {0}")]
    Parse(String),

    #[error("{0}")]
    Invalid(String),
}

impl GitError {
    /// Stable machine-readable kind, used by the IPC layer.
    pub fn kind(&self) -> &'static str {
        match self {
            GitError::GitNotFound(_) => "gitNotFound",
            GitError::Io(_) => "io",
            GitError::Failed { .. } => "gitFailed",
            GitError::NotARepository(_) => "notARepository",
            GitError::Cancelled => "cancelled",
            GitError::TimedOut => "timedOut",
            GitError::Parse(_) => "parse",
            GitError::Invalid(_) => "invalid",
        }
    }
}
