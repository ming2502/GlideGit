//! Parsers for machine-readable git output formats.

use crate::error::{GitError, Result};
use crate::model::*;

// ---------------------------------------------------------------------------
// git status --porcelain=v2 -z --branch
// ---------------------------------------------------------------------------

pub fn parse_status_v2(out: &[u8]) -> Result<(HeadInfo, Vec<FileChange>)> {
    let text = String::from_utf8_lossy(out);
    let mut head = HeadInfo::default();
    let mut changes = Vec::new();
    let mut tokens = text.split('\0').filter(|t| !t.is_empty());

    while let Some(tok) = tokens.next() {
        if let Some(header) = tok.strip_prefix("# ") {
            let (key, val) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" => head.oid = (val != "(initial)").then(|| val.to_string()),
                "branch.head" => {
                    if val == "(detached)" {
                        head.detached = true;
                    } else {
                        head.branch = Some(val.to_string());
                    }
                }
                "branch.upstream" => head.upstream = Some(val.to_string()),
                "branch.ab" => {
                    for part in val.split(' ') {
                        if let Some(n) = part.strip_prefix('+') {
                            head.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            head.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }

        match tok.as_bytes()[0] {
            b'1' => {
                // 1 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <path>
                let f: Vec<&str> = tok.splitn(9, ' ').collect();
                if f.len() < 9 {
                    return Err(GitError::Parse(format!("status entry: {tok}")));
                }
                changes.push(ordinary(f[1], f[8], None));
            }
            b'2' => {
                // 2 <XY> <sub> <mH> <mI> <mW> <hH> <hI> <X><score> <path>\0<origPath>
                let f: Vec<&str> = tok.splitn(10, ' ').collect();
                if f.len() < 10 {
                    return Err(GitError::Parse(format!("status entry: {tok}")));
                }
                let orig = tokens.next().map(str::to_string);
                changes.push(ordinary(f[1], f[9], orig));
            }
            b'u' => {
                // u <XY> <sub> <m1> <m2> <m3> <mW> <h1> <h2> <h3> <path>
                let f: Vec<&str> = tok.splitn(11, ' ').collect();
                if f.len() < 11 {
                    return Err(GitError::Parse(format!("status entry: {tok}")));
                }
                changes.push(FileChange {
                    path: f[10].to_string(),
                    orig_path: None,
                    staged: None,
                    unstaged: Some(ChangeKind::Conflicted),
                    conflicted: true,
                });
            }
            b'?' => changes.push(FileChange {
                path: tok.get(2..).unwrap_or_default().to_string(),
                orig_path: None,
                staged: None,
                unstaged: Some(ChangeKind::Untracked),
                conflicted: false,
            }),
            b'!' => {}
            _ => return Err(GitError::Parse(format!("status entry: {tok}"))),
        }
    }
    Ok((head, changes))
}

fn ordinary(xy: &str, path: &str, orig_path: Option<String>) -> FileChange {
    let mut chars = xy.chars();
    let x = chars.next().unwrap_or('.');
    let y = chars.next().unwrap_or('.');
    FileChange {
        path: path.to_string(),
        orig_path,
        staged: change_kind(x),
        unstaged: change_kind(y),
        conflicted: false,
    }
}

fn change_kind(c: char) -> Option<ChangeKind> {
    Some(match c {
        '.' | ' ' => return None,
        'A' => ChangeKind::Added,
        'D' => ChangeKind::Deleted,
        'R' => ChangeKind::Renamed,
        'C' => ChangeKind::Copied,
        'T' => ChangeKind::TypeChanged,
        'U' => ChangeKind::Conflicted,
        _ => ChangeKind::Modified,
    })
}

// ---------------------------------------------------------------------------
// git for-each-ref
// ---------------------------------------------------------------------------

pub const FOR_EACH_REF_FORMAT: &str = "--format=%(refname)%00%(refname:short)%00%(objectname)%00%(upstream:short)%00%(upstream:track,nobracket)%00%(HEAD)%00%(committerdate:unix)%00%(contents:subject)%00%(symref)";

/// Returns `(local, remote)` branches in the order git printed them.
pub fn parse_for_each_ref(text: &str) -> (Vec<Branch>, Vec<Branch>) {
    let mut local = Vec::new();
    let mut remote = Vec::new();
    for line in text.lines() {
        let f: Vec<&str> = line.split('\0').collect();
        if f.len() < 9 {
            continue;
        }
        // Skip symbolic refs such as refs/remotes/origin/HEAD.
        if !f[8].is_empty() {
            continue;
        }
        let full_ref = f[0];
        let is_remote = full_ref.starts_with("refs/remotes/");
        let (ahead, behind, gone) = parse_track(f[4]);
        let branch = Branch {
            name: f[1].to_string(),
            full_ref: full_ref.to_string(),
            oid: f[2].to_string(),
            upstream: (!f[3].is_empty()).then(|| f[3].to_string()),
            upstream_gone: gone,
            ahead,
            behind,
            is_head: f[5] == "*",
            is_remote,
            commit_time: f[6].parse().unwrap_or(0),
            subject: f[7].to_string(),
        };
        if is_remote {
            remote.push(branch);
        } else {
            local.push(branch);
        }
    }
    (local, remote)
}

/// `ahead 3, behind 2` | `ahead 3` | `behind 2` | `gone` | ``
fn parse_track(s: &str) -> (u32, u32, bool) {
    if s == "gone" {
        return (0, 0, true);
    }
    let (mut ahead, mut behind) = (0, 0);
    for part in s.split(", ") {
        if let Some(n) = part.strip_prefix("ahead ") {
            ahead = n.parse().unwrap_or(0);
        } else if let Some(n) = part.strip_prefix("behind ") {
            behind = n.parse().unwrap_or(0);
        }
    }
    (ahead, behind, false)
}

/// Extract recently checked-out branch names from `git reflog --format=%gs`.
pub fn parse_recent_branches(reflog: &str, local: &[Branch], limit: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in reflog.lines() {
        let Some(rest) = line.strip_prefix("checkout: moving from ") else {
            continue;
        };
        let Some((_, to)) = rest.rsplit_once(" to ") else {
            continue;
        };
        if out.iter().any(|b| b == to) {
            continue;
        }
        if local.iter().any(|b| b.name == to) {
            out.push(to.to_string());
            if out.len() >= limit {
                break;
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// git log
// ---------------------------------------------------------------------------

pub const LOG_FORMAT: &str = "--format=%H%x00%P%x00%an%x00%ae%x00%at%x00%s%x00%D%x1e";

pub fn parse_log(text: &str) -> Vec<CommitSummary> {
    text.split('\x1e')
        .map(|r| r.trim_start_matches('\n'))
        .filter(|r| !r.is_empty())
        .filter_map(|r| {
            let f: Vec<&str> = r.split('\0').collect();
            if f.len() < 7 {
                return None;
            }
            Some(CommitSummary {
                oid: f[0].to_string(),
                parents: f[1].split_whitespace().map(str::to_string).collect(),
                author_name: f[2].to_string(),
                author_email: f[3].to_string(),
                author_time: f[4].parse().unwrap_or(0),
                subject: f[5].to_string(),
                refs: f[6]
                    .split(", ")
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
                    .collect(),
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// unified diff
// ---------------------------------------------------------------------------

pub fn parse_unified_diff(text: &str, path: &str, max_lines: usize) -> FileDiff {
    let mut diff = FileDiff {
        path: path.to_string(),
        ..FileDiff::default()
    };
    let mut hunk: Option<DiffHunk> = None;
    let mut combined = false;
    let (mut old_no, mut new_no) = (0u32, 0u32);
    let mut total = 0usize;

    for raw in text.split('\n') {
        let line = raw.strip_suffix('\r').unwrap_or(raw);

        if line.starts_with("@@") {
            if let Some(h) = hunk.take() {
                diff.hunks.push(h);
            }
            combined = line.starts_with("@@@");
            match parse_hunk_header(line) {
                Some(h) => {
                    old_no = h.old_start;
                    new_no = h.new_start;
                    hunk = Some(h);
                }
                None => hunk = None,
            }
            continue;
        }

        let Some(h) = hunk.as_mut() else {
            // File header area.
            if line.starts_with("Binary files ") || line == "GIT binary patch" {
                diff.binary = true;
            }
            continue;
        };

        if total >= max_lines {
            diff.truncated = true;
            break;
        }

        let prefix_len = if combined { 2 } else { 1 };
        if line.is_empty() {
            // Trailing empty line at the end of output.
            continue;
        }
        if let Some(rest) = line.strip_prefix('\\') {
            h.lines.push(DiffLine {
                kind: LineKind::NoNewline,
                old_line: None,
                new_line: None,
                text: rest.trim().to_string(),
            });
            continue;
        }
        let prefix = line.get(..prefix_len).unwrap_or(line);
        let body = line.get(prefix_len..).unwrap_or("").to_string();
        let kind = if prefix.contains('+') {
            LineKind::Added
        } else if prefix.contains('-') {
            LineKind::Deleted
        } else {
            LineKind::Context
        };
        let (o, n) = match kind {
            LineKind::Added => {
                new_no += 1;
                diff.added += 1;
                (None, Some(new_no - 1))
            }
            LineKind::Deleted => {
                old_no += 1;
                diff.deleted += 1;
                (Some(old_no - 1), None)
            }
            _ => {
                old_no += 1;
                new_no += 1;
                (Some(old_no - 1), Some(new_no - 1))
            }
        };
        h.lines.push(DiffLine {
            kind,
            old_line: o,
            new_line: n,
            text: body,
        });
        total += 1;
    }
    if let Some(h) = hunk {
        diff.hunks.push(h);
    }
    diff
}

/// Parses `@@ -a,b +c,d @@ ctx` and combined `@@@ -a,b -c,d +e,f @@@ ctx`.
fn parse_hunk_header(line: &str) -> Option<DiffHunk> {
    let marker = if line.starts_with("@@@") { "@@@" } else { "@@" };
    let inner = line.strip_prefix(marker)?;
    let end = inner.find(marker)?;
    let ranges = inner[..end].trim();
    let mut old = (0, 0);
    let mut new = (0, 0);
    for r in ranges.split(' ') {
        if let Some(v) = r.strip_prefix('-') {
            // For combined diffs keep the first parent.
            if old == (0, 0) {
                old = parse_range(v)?;
            }
        } else if let Some(v) = r.strip_prefix('+') {
            new = parse_range(v)?;
        }
    }
    Some(DiffHunk {
        header: line.to_string(),
        old_start: old.0,
        old_lines: old.1,
        new_start: new.0,
        new_lines: new.1,
        lines: Vec::new(),
    })
}

fn parse_range(s: &str) -> Option<(u32, u32)> {
    match s.split_once(',') {
        Some((a, b)) => Some((a.parse().ok()?, b.parse().ok()?)),
        None => Some((s.parse().ok()?, 1)),
    }
}

/// Build an all-added diff for a file git does not track yet.
pub fn synthesize_added(path: &str, content: &[u8], max_lines: usize) -> FileDiff {
    let mut diff = FileDiff {
        path: path.to_string(),
        ..FileDiff::default()
    };
    let probe = &content[..content.len().min(8000)];
    if probe.contains(&0) {
        diff.binary = true;
        return diff;
    }
    if content.is_empty() {
        return diff;
    }
    let text = String::from_utf8_lossy(content);
    let mut lines = Vec::new();
    let body = text.strip_suffix('\n').unwrap_or(&text);
    for (i, l) in body.split('\n').enumerate() {
        if i >= max_lines {
            diff.truncated = true;
            break;
        }
        lines.push(DiffLine {
            kind: LineKind::Added,
            old_line: None,
            new_line: Some(i as u32 + 1),
            text: l.strip_suffix('\r').unwrap_or(l).to_string(),
        });
    }
    if !text.ends_with('\n') && !diff.truncated {
        lines.push(DiffLine {
            kind: LineKind::NoNewline,
            old_line: None,
            new_line: None,
            text: "No newline at end of file".into(),
        });
    }
    let n = lines.iter().filter(|l| l.kind == LineKind::Added).count() as u32;
    diff.added = n;
    diff.hunks.push(DiffHunk {
        header: format!("@@ -0,0 +1,{n} @@"),
        old_start: 0,
        old_lines: 0,
        new_start: 1,
        new_lines: n,
        lines,
    });
    diff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_v2_full() {
        let raw = "# branch.oid 1234abcd\0# branch.head main\0# branch.upstream origin/main\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 aaa bbb src/a file.rs\0\
1 MM N... 100644 100644 100644 aaa bbb both.txt\0\
2 R. N... 100644 100644 100644 aaa bbb R100 new name.txt\0old name.txt\0\
u UU N... 100644 100644 100644 100644 a b c conflict.txt\0\
? 新文件.md\0";
        let (head, ch) = parse_status_v2(raw.as_bytes()).unwrap();
        assert_eq!(head.branch.as_deref(), Some("main"));
        assert_eq!(head.oid.as_deref(), Some("1234abcd"));
        assert_eq!((head.ahead, head.behind), (2, 1));
        assert_eq!(head.upstream.as_deref(), Some("origin/main"));
        assert_eq!(ch.len(), 5);
        assert_eq!(ch[0].path, "src/a file.rs");
        assert_eq!(ch[0].staged, None);
        assert_eq!(ch[0].unstaged, Some(ChangeKind::Modified));
        assert_eq!(ch[1].staged, Some(ChangeKind::Modified));
        assert_eq!(ch[2].path, "new name.txt");
        assert_eq!(ch[2].orig_path.as_deref(), Some("old name.txt"));
        assert_eq!(ch[2].staged, Some(ChangeKind::Renamed));
        assert!(ch[3].conflicted);
        assert_eq!(ch[4].path, "新文件.md");
        assert!(ch[4].is_untracked());
    }

    #[test]
    fn status_v2_unborn_detached() {
        let raw = "# branch.oid (initial)\0# branch.head (detached)\0";
        let (head, ch) = parse_status_v2(raw.as_bytes()).unwrap();
        assert!(head.oid.is_none());
        assert!(head.detached);
        assert!(head.branch.is_none());
        assert!(ch.is_empty());
    }

    #[test]
    fn for_each_ref() {
        let raw =
            "refs/heads/main\0main\0abc\0origin/main\0ahead 1, behind 2\0*\x001700000000\0init\0\n\
refs/heads/old\0old\0def\0origin/old\0gone\0 \x001600000000\0x\0\n\
refs/remotes/origin/HEAD\0origin\0abc\0\0\0 \x001700000000\0init\0refs/remotes/origin/main\n\
refs/remotes/origin/main\0origin/main\0abc\0\0\0 \x001700000000\0init\0\n";
        let (local, remote) = parse_for_each_ref(raw);
        assert_eq!(local.len(), 2);
        assert_eq!(remote.len(), 1);
        assert!(local[0].is_head);
        assert_eq!((local[0].ahead, local[0].behind), (1, 2));
        assert!(local[1].upstream_gone);
        assert_eq!(remote[0].name, "origin/main");
    }

    #[test]
    fn recent_branches() {
        let local: Vec<Branch> = ["main", "feat", "fix"]
            .iter()
            .map(|n| Branch {
                name: n.to_string(),
                full_ref: format!("refs/heads/{n}"),
                oid: String::new(),
                upstream: None,
                upstream_gone: false,
                ahead: 0,
                behind: 0,
                is_head: false,
                is_remote: false,
                commit_time: 0,
                subject: String::new(),
            })
            .collect();
        let reflog = "checkout: moving from feat to main\ncommit: x\ncheckout: moving from main to feat\ncheckout: moving from fix to main\ncheckout: moving from main to deleted\n";
        assert_eq!(
            parse_recent_branches(reflog, &local, 10),
            vec!["main", "feat"]
        );
    }

    #[test]
    fn log() {
        let raw = "aaa\0p1 p2\0Ann\0a@x\x001700000000\0Merge stuff\0HEAD -> main, origin/main\x1e\nbbb\0\0Bob\0b@x\x001600000000\0init\0\x1e\n";
        let c = parse_log(raw);
        assert_eq!(c.len(), 2);
        assert_eq!(c[0].parents, vec!["p1", "p2"]);
        assert_eq!(c[0].refs, vec!["HEAD -> main", "origin/main"]);
        assert!(c[1].parents.is_empty());
        assert!(c[1].refs.is_empty());
    }

    #[test]
    fn unified_diff() {
        let raw = "diff --git a/f b/f\nindex 1..2 100644\n--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@ fn main\n a\n-b\n+B\n c\n@@ -10 +10,2 @@\n x\n+y\n\\ No newline at end of file\n";
        let d = parse_unified_diff(raw, "f", 10_000);
        assert_eq!(d.hunks.len(), 2);
        assert_eq!((d.added, d.deleted), (2, 1));
        let h = &d.hunks[0];
        assert_eq!(
            (h.old_start, h.old_lines, h.new_start, h.new_lines),
            (1, 3, 1, 3)
        );
        assert_eq!(h.lines[1].kind, LineKind::Deleted);
        assert_eq!(h.lines[1].old_line, Some(2));
        assert_eq!(h.lines[2].new_line, Some(2));
        assert_eq!(d.hunks[1].old_lines, 1);
        assert_eq!(d.hunks[1].lines[2].kind, LineKind::NoNewline);
    }

    #[test]
    fn binary_diff() {
        let raw = "diff --git a/x.png b/x.png\nindex 1..2 100644\nBinary files a/x.png and b/x.png differ\n";
        assert!(parse_unified_diff(raw, "x.png", 100).binary);
    }

    #[test]
    fn truncation() {
        let mut raw = String::from("@@ -0,0 +1,100 @@\n");
        for i in 0..100 {
            raw.push_str(&format!("+{i}\n"));
        }
        let d = parse_unified_diff(&raw, "f", 10);
        assert!(d.truncated);
        assert_eq!(d.hunks[0].lines.len(), 10);
    }

    #[test]
    fn synthesized() {
        let d = synthesize_added("a", b"one\r\ntwo", 100);
        assert_eq!(d.added, 2);
        assert_eq!(d.hunks[0].lines[0].text, "one");
        assert_eq!(d.hunks[0].lines[2].kind, LineKind::NoNewline);
        assert!(synthesize_added("b", b"\0\x01", 100).binary);
    }
}
