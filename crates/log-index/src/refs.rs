//! The commits the index walks from, and the ref labels shown on rows.

use std::collections::{BTreeSet, HashMap};

use glide_git_backend::{Repo, RunOptions};

use crate::Result;

const FORMAT: &str = "--format=%(objectname)%00%(objecttype)%00%(*objectname)%00%(*objecttype)%00%(refname)%00%(committerdate:unix)%00%(*committerdate:unix)";

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RefTips {
    /// Commit ids reachable history starts from: branches, remote branches,
    /// tags and HEAD. Sorted and deduplicated, so equal sets compare equal.
    pub tips: Vec<String>,
    /// Commit id → labels in `git log --decorate` style
    /// (`HEAD -> main`, `origin/main`, `tag: v1.0`).
    pub labels: HashMap<String, Vec<String>>,
    /// Commit id → committer time, where known (for the quick first page).
    pub times: HashMap<String, i64>,
}

/// One `for-each-ref` call plus reading `HEAD` directly: this runs after every
/// refresh, so it has to stay cheap.
pub fn read_tips(repo: &Repo) -> Result<RefTips> {
    let out = repo.cli().run(
        repo.root(),
        &[
            "for-each-ref",
            FORMAT,
            "refs/heads",
            "refs/remotes",
            "refs/tags",
        ],
        RunOptions::read(),
    )?;
    let head = std::fs::read_to_string(repo.git_dir().join("HEAD")).unwrap_or_default();
    Ok(parse(&head, &out.stdout_str()))
}

pub fn parse(head_file: &str, for_each_ref: &str) -> RefTips {
    let head = head_file.trim();
    let head_ref = head.strip_prefix("ref: ").map(str::trim);

    let mut tips = BTreeSet::new();
    let mut labels: HashMap<String, Vec<String>> = HashMap::new();
    let mut times: HashMap<String, i64> = HashMap::new();
    let mut head_label = None;

    for line in for_each_ref.lines() {
        let f: Vec<&str> = line.split('\0').collect();
        if f.len() < 5 {
            continue;
        }
        let refname = f[4];
        // Annotated tags point at a tag object; label the commit it peels to.
        let commit = match (f[1], f[3]) {
            ("commit", _) => f[0],
            ("tag", "commit") => f[2],
            _ => continue,
        };
        let label = if let Some(b) = refname.strip_prefix("refs/heads/") {
            if head_ref == Some(refname) {
                head_label = Some(commit.to_string());
                format!("HEAD -> {b}")
            } else {
                b.to_string()
            }
        } else if let Some(r) = refname.strip_prefix("refs/remotes/") {
            // `origin/HEAD` is a pointer to the default branch, not a branch.
            if r.ends_with("/HEAD") {
                tips.insert(commit.to_string());
                continue;
            }
            r.to_string()
        } else if let Some(t) = refname.strip_prefix("refs/tags/") {
            format!("tag: {t}")
        } else {
            continue;
        };
        tips.insert(commit.to_string());
        let time = if f[1] == "commit" { f.get(5) } else { f.get(6) };
        if let Some(t) = time.and_then(|t| t.parse().ok()) {
            times.insert(commit.to_string(), t);
        }
        labels.entry(commit.to_string()).or_default().push(label);
    }

    // Detached HEAD: the file holds the commit id itself.
    if head_ref.is_none() && !head.is_empty() && head.bytes().all(|b| b.is_ascii_hexdigit()) {
        tips.insert(head.to_string());
        labels
            .entry(head.to_string())
            .or_default()
            .insert(0, "HEAD".into());
    } else if let Some(oid) = head_label {
        // Keep `HEAD -> branch` first, like git does.
        if let Some(l) = labels.get_mut(&oid) {
            if let Some(i) = l.iter().position(|x| x.starts_with("HEAD -> ")) {
                let h = l.remove(i);
                l.insert(0, h);
            }
        }
    }

    RefTips {
        tips: tips.into_iter().collect(),
        labels,
        times,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "1111111111111111111111111111111111111111";
    const B: &str = "2222222222222222222222222222222222222222";
    const T: &str = "3333333333333333333333333333333333333333";

    fn line(oid: &str, ty: &str, peeled: &str, pty: &str, name: &str) -> String {
        format!("{oid}\0{ty}\0{peeled}\0{pty}\0{name}\n")
    }

    #[test]
    fn labels_and_tips() {
        let refs = [
            line(B, "commit", "", "", "refs/heads/feature"),
            line(A, "commit", "", "", "refs/heads/main"),
            line(A, "commit", "", "", "refs/remotes/origin/HEAD"),
            line(A, "commit", "", "", "refs/remotes/origin/main"),
            line(T, "tag", B, "commit", "refs/tags/v1"),
            line(A, "commit", "", "", "refs/tags/light"),
            line(T, "tag", "4444", "tree", "refs/tags/tree-tag"),
        ]
        .concat();
        let r = parse("ref: refs/heads/main\n", &refs);
        assert_eq!(r.tips, vec![A.to_string(), B.to_string()]);
        assert_eq!(
            r.labels[A],
            vec!["HEAD -> main", "origin/main", "tag: light"]
        );
        assert_eq!(r.labels[B], vec!["feature", "tag: v1"]);
    }

    #[test]
    fn detached_and_unborn_head() {
        let refs = line(A, "commit", "", "", "refs/heads/main");
        let r = parse(&format!("{B}\n"), &refs);
        assert_eq!(r.tips, vec![A.to_string(), B.to_string()]);
        assert_eq!(r.labels[B], vec!["HEAD"]);
        assert_eq!(r.labels[A], vec!["main"]);

        let r = parse("ref: refs/heads/main\n", "");
        assert!(r.tips.is_empty());
    }
}
