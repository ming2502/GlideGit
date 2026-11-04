//! GlideGit git backend: runs the system `git`, parses its machine-readable
//! output and exposes a typed repository API.

mod cli;
mod error;
mod model;
mod parse;
mod repo;

pub use cli::{CancelToken, GitCli, GitOutput, RunOptions};
pub use error::{GitError, Result};
pub use model::*;
pub use repo::{PullMode, Repo, MAX_DIFF_LINES};
