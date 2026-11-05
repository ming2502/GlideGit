//! GlideGit core: repository sessions, write serialisation, file watching
//! and snapshot publication. UI-agnostic; the Tauri app is a thin shell on top.

pub mod askpass;
mod cache;
mod error;
mod manager;
pub mod oplog;
mod session;

pub use error::{CoreError, Result};
pub use glide_log_index::{AuthorCount, LogFilter, LogPage, LogStatus};
pub use manager::RepositoryManager;
pub use oplog::{OpEntry, OpKind, OpMeta};
pub use session::{is_relevant, RepoSession, SharedSnapshot, DEBOUNCE};
