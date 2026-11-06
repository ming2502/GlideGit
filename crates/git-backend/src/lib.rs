//! GlideGit git backend: runs the system `git`, parses its machine-readable
//! output and exposes a typed repository API.

mod cli;
mod error;
pub mod i18n;
mod model;
mod parse;
mod repo;

pub use cli::{CancelToken, GitCli, GitOutput, ProgressSink, RunOptions};
pub use error::{GitError, Result};
pub use model::*;
pub use repo::{NetOptions, PullMode, Repo, ResetMode, MAX_DIFF_LINES, SHELF_REFS};
