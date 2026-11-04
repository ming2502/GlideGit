//! Log filters: which commits a filtered Log shows.
//!
//! Text, author and date conditions are answered from the index with SQL.
//! Branch and path conditions need git (`rev-list`, `log -- <path>`); git
//! returns a set of commit ids that the SQL results are then checked against.

use std::collections::HashSet;

use glide_git_backend::{CancelToken, GitError, Repo, RunOptions};
use rusqlite::types::Value;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::record::hex_decode;
use crate::Result;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", default)]
#[ts(export)]
pub struct LogFilter {
    /// Matches the subject (case-insensitive for ASCII) or, when it looks
    /// like one, a commit id prefix.
    pub text: String,
    /// Only history reachable from these refs (`refs/heads/main`,
    /// `refs/remotes/origin/main`, `HEAD`, …).
    pub branches: Vec<String>,
    /// Matches `Name <email>` by substring; any of them.
    pub authors: Vec<String>,
    /// Author time bounds, unix seconds, inclusive.
    pub since: Option<i64>,
    pub until: Option<i64>,
    /// Only commits touching these paths (relative to the repository root).
    pub paths: Vec<String>,
}

/// An author and how many indexed commits they wrote.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export)]
pub struct AuthorCount {
    pub name: String,
    pub email: String,
    pub commits: u32,
}

impl LogFilter {
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
            && self.branches.is_empty()
            && self.authors.is_empty()
            && self.since.is_none()
            && self.until.is_none()
            && self.paths.is_empty()
    }

    /// Whether the result still has a meaningful graph. History reachable
    /// from some branches is closed under parents, so it can be laid out like
    /// the full Log; any other condition leaves holes in the history.
    pub fn keeps_graph(&self) -> bool {
        self.text.trim().is_empty()
            && self.authors.is_empty()
            && self.since.is_none()
            && self.until.is_none()
            && self.paths.is_empty()
    }

    /// SQL condition over `commits c JOIN authors a`, with its parameters.
    pub(crate) fn sql(&self) -> (String, Vec<Value>) {
        let mut conds = Vec::new();
        let mut params = Vec::new();
        let text = self.text.trim();
        if !text.is_empty() {
            let mut alts = vec![format!("c.subject LIKE ?{} ESCAPE '\\'", params.len() + 1)];
            params.push(Value::Text(like_pattern(text)));
            if let Some((lo, hi)) = hash_range(text) {
                alts.push(format!(
                    "(c.oid >= ?{} AND c.oid <= ?{})",
                    params.len() + 1,
                    params.len() + 2
                ));
                params.push(Value::Blob(lo));
                params.push(Value::Blob(hi));
            }
            conds.push(format!("({})", alts.join(" OR ")));
        }
        if !self.authors.is_empty() {
            let alts: Vec<String> = self
                .authors
                .iter()
                .map(|a| {
                    params.push(Value::Text(like_pattern(a.trim())));
                    format!(
                        "(a.name || ' <' || a.email || '>') LIKE ?{} ESCAPE '\\'",
                        params.len()
                    )
                })
                .collect();
            conds.push(format!("({})", alts.join(" OR ")));
        }
        if let Some(t) = self.since {
            params.push(Value::Integer(t));
            conds.push(format!("c.time >= ?{}", params.len()));
        }
        if let Some(t) = self.until {
            params.push(Value::Integer(t));
            conds.push(format!("c.time <= ?{}", params.len()));
        }
        if conds.is_empty() {
            ("1".into(), params)
        } else {
            (conds.join(" AND "), params)
        }
    }

    /// Commit ids allowed by the branch and path conditions; `None` when
    /// there are none. `all_tips` is used for a path filter without branches.
    pub(crate) fn git_set(
        &self,
        repo: &Repo,
        all_tips: &[String],
        cancel: &CancelToken,
    ) -> Result<Option<HashSet<Vec<u8>>>> {
        if self.branches.is_empty() && self.paths.is_empty() {
            return Ok(None);
        }
        for b in &self.branches {
            // Revisions go to git on stdin, where a leading '-' is an option.
            if b.is_empty() || b.starts_with('-') || b.chars().any(char::is_whitespace) {
                return Err(GitError::Invalid(format!("not a branch: {b:?}")).into());
            }
        }
        let revs: &[String] = if self.branches.is_empty() {
            all_tips
        } else {
            &self.branches
        };
        if revs.is_empty() {
            return Ok(Some(HashSet::new()));
        }
        let mut input = revs.join("\n");
        input.push('\n');

        let mut args = vec!["log", "--format=%H", "--stdin"];
        if self.paths.is_empty() {
            args = vec!["rev-list", "--stdin"];
        } else {
            args.push("--");
            args.extend(self.paths.iter().map(String::as_str));
        }
        let mut set = HashSet::new();
        let mut rest = Vec::new();
        repo.cli().run_streaming(
            repo.root(),
            &args,
            RunOptions::read().stdin(input).cancel(cancel.clone()),
            |chunk| {
                rest.extend_from_slice(chunk);
                let mut start = 0;
                while let Some(i) = rest[start..].iter().position(|&b| b == b'\n') {
                    if let Some(oid) = std::str::from_utf8(&rest[start..start + i])
                        .ok()
                        .and_then(hex_decode)
                    {
                        set.insert(oid);
                    }
                    start += i + 1;
                }
                rest.drain(..start);
                true
            },
        )?;
        Ok(Some(set))
    }
}

/// `%text%` with LIKE wildcards in `text` escaped.
pub(crate) fn like_pattern(text: &str) -> String {
    let mut p = String::with_capacity(text.len() + 2);
    p.push('%');
    for ch in text.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            p.push('\\');
        }
        p.push(ch);
    }
    p.push('%');
    p
}

/// Blob bounds covering every id that starts with the hex prefix `text`
/// (at least 4 hex digits, like git's shortest abbreviation).
fn hash_range(text: &str) -> Option<(Vec<u8>, Vec<u8>)> {
    if text.len() < 4 || text.len() > 64 || !text.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let odd = !text.len().is_multiple_of(2);
    let lo = hex_decode(&format!("{text}{}", if odd { "0" } else { "" }))?;
    let mut hi = hex_decode(&format!("{text}{}", if odd { "f" } else { "" }))?;
    // Longer ids with this prefix compare greater than the prefix itself.
    hi.extend([0xff; 32]);
    Some((lo, hi))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn like_escaping() {
        assert_eq!(like_pattern("50%_a\\b"), "%50\\%\\_a\\\\b%");
    }

    #[test]
    fn hash_ranges() {
        assert_eq!(hash_range("abc"), None);
        assert_eq!(hash_range("fix bug"), None);
        let (lo, hi) = hash_range("abcd1").unwrap();
        assert_eq!(lo, vec![0xab, 0xcd, 0x10]);
        assert_eq!(&hi[..3], &[0xab, 0xcd, 0x1f]);
        let id = [0xab, 0xcd, 0x1e, 0x00];
        assert!(lo.as_slice() <= &id[..] && &id[..] <= hi.as_slice());
    }

    #[test]
    fn graph_only_for_branch_filters() {
        let mut f = LogFilter {
            branches: vec!["refs/heads/main".into()],
            ..LogFilter::default()
        };
        assert!(f.keeps_graph() && !f.is_empty());
        f.paths.push("src".into());
        assert!(!f.keeps_graph());
        assert!(LogFilter::default().is_empty());
    }
}
