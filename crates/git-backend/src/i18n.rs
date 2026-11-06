//! The interface language for text the backend writes for people (operation
//! titles, undo descriptions, some errors). The UI sets it at start-up; git's
//! own messages stay as git prints them.

use std::sync::atomic::{AtomicBool, Ordering};

static ENGLISH: AtomicBool = AtomicBool::new(false);

pub fn set_english(on: bool) {
    ENGLISH.store(on, Ordering::Relaxed);
}

pub fn english() -> bool {
    ENGLISH.load(Ordering::Relaxed)
}

/// Chinese or English text, formatted like `format!` (arguments, including
/// captured identifiers, are shared by both).
///
/// ```
/// use glide_git_backend::{i18n, tr};
/// let n = 3;
/// assert_eq!(tr!("回滚 {n} 个文件", "Roll back {n} files"), "回滚 3 个文件");
/// i18n::set_english(true);
/// assert_eq!(tr!("回滚 {n} 个文件", "Roll back {n} files"), "Roll back 3 files");
/// assert_eq!(tr!("回到 {}", "go back to {}", "abc"), "go back to abc");
/// ```
#[macro_export]
macro_rules! tr {
    ($zh:literal, $en:literal $(, $args:expr)* $(,)?) => {
        if $crate::i18n::english() {
            format!($en $(, $args)*)
        } else {
            format!($zh $(, $args)*)
        }
    };
}
