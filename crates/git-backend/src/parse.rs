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
// --progress lines
// ---------------------------------------------------------------------------

/// Parses `[remote: ]<Phase>: [NN% ]<detail>[, done.]`. Returns `None` for
/// anything that is not a progress line (errors, `To <url>`, ref updates…).
pub fn parse_progress_line(line: &str) -> Option<Progress> {
    let line = line.trim_end();
    let line = line.strip_prefix("remote: ").unwrap_or(line).trim();
    let (phase, rest) = line.split_once(": ")?;
    // Phases are short capitalised phrases; rejects `error: …`, `fatal: …`, `hint: …`.
    if phase.is_empty()
        || phase.len() > 40
        || !phase.starts_with(|c: char| c.is_ascii_uppercase())
        || !phase.chars().all(|c| c.is_ascii_alphabetic() || c == ' ')
    {
        return None;
    }
    let rest = rest.trim();
    let (percent, detail) = match rest.split_once('%') {
        Some((num, after)) if !num.is_empty() && num.trim().chars().all(|c| c.is_ascii_digit()) => {
            (num.trim().parse::<u8>().ok(), after.trim())
        }
        _ => {
            // `Enumerating objects: 5, done.` — a count, no percentage.
            if !rest.starts_with(|c: char| c.is_ascii_digit()) {
                return None;
            }
            (None, rest)
        }
    };
    let detail = detail
        .trim_end_matches(", done.")
        .trim_end_matches(", done")
        .trim()
        .to_string();
    Some(Progress {
        phase: phase.to_string(),
        percent,
        detail,
    })
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

/// Parse `git blame --porcelain`. Each line comes as a header
/// (`<oid> <orig> <final> [<count>]`), commit details the first time a
/// commit appears, and the line itself prefixed with a tab.
pub fn parse_blame(out: &[u8], path: &str) -> Blame {
    let text = String::from_utf8_lossy(out);
    let mut blame = Blame {
        path: path.to_string(),
        ..Blame::default()
    };
    let mut index: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut current: Option<u32> = None;
    let mut content = String::new();
    for line in text.split_inclusive('\n') {
        if let Some(code) = line.strip_prefix('\t') {
            if let Some(c) = current {
                blame.lines.push(c);
                content.push_str(code);
            }
            continue;
        }
        let line = line.trim_end_matches('\n');
        let (key, value) = line.split_once(' ').unwrap_or((line, ""));
        let is_header = key.len() >= 40 && key.bytes().all(|b| b.is_ascii_hexdigit());
        if is_header {
            let id = *index.entry(key.to_string()).or_insert_with(|| {
                blame.commits.push(BlameCommit {
                    oid: key.to_string(),
                    ..BlameCommit::default()
                });
                (blame.commits.len() - 1) as u32
            });
            current = Some(id);
            continue;
        }
        let Some(c) = current.and_then(|i| blame.commits.get_mut(i as usize)) else {
            continue;
        };
        match key {
            "author" => c.author_name = value.to_string(),
            "author-mail" => {
                c.author_email = value.trim_matches(|ch| ch == '<' || ch == '>').to_string()
            }
            "author-time" => c.author_time = value.parse().unwrap_or(0),
            "summary" => c.summary = value.to_string(),
            "previous" => {
                if let Some((oid, prev)) = value.split_once(' ') {
                    c.previous_oid = Some(oid.to_string());
                    c.previous_path = Some(prev.to_string());
                }
            }
            _ => {}
        }
    }
    blame.text = content;
    blame
}

pub const STASH_FORMAT: &str = "--format=%gd%x00%H%x00%gs%x00%ct%x1e";

/// Parse `git stash list` in [`STASH_FORMAT`].
pub fn parse_stash_list(text: &str) -> Vec<StashEntry> {
    text.split('\x1e')
        .map(|r| r.trim_start_matches('\n'))
        .filter(|r| !r.is_empty())
        .filter_map(|r| {
            let f: Vec<&str> = r.split('\0').collect();
            if f.len() < 4 {
                return None;
            }
            let index = f[0]
                .strip_prefix("stash@{")?
                .strip_suffix('}')?
                .parse()
                .ok()?;
            // "WIP on main: 1a2b3c subject" or "On main: my message".
            let subject = f[2];
            let (branch, message) = match subject
                .strip_prefix("WIP on ")
                .or_else(|| subject.strip_prefix("On "))
                .and_then(|rest| rest.split_once(": "))
            {
                Some((b, m)) => (Some(b.to_string()), m.to_string()),
                None => (None, subject.to_string()),
            };
            Some(StashEntry {
                index,
                oid: f[1].to_string(),
                message,
                branch: branch.filter(|b| b != "(no branch)"),
                time: f[3].trim().parse().unwrap_or(0),
            })
        })
        .collect()
}

/// `git log -1` format for [`CommitDetails`] (files are filled in separately).
pub const DETAILS_FORMAT: &str =
    "--format=%H%x00%P%x00%an%x00%ae%x00%at%x00%cn%x00%ce%x00%ct%x00%B";

pub fn parse_details(text: &str) -> Option<CommitDetails> {
    let f: Vec<&str> = text.splitn(9, '\0').collect();
    if f.len() < 9 {
        return None;
    }
    Some(CommitDetails {
        oid: f[0].trim().to_string(),
        parents: f[1].split_whitespace().map(str::to_string).collect(),
        author_name: f[2].to_string(),
        author_email: f[3].to_string(),
        author_time: f[4].parse().unwrap_or(0),
        committer_name: f[5].to_string(),
        committer_email: f[6].to_string(),
        committer_time: f[7].parse().unwrap_or(0),
        message: f[8].trim_end().to_string(),
        files: Vec::new(),
    })
}

/// Parse `git diff-tree -r -z -M --raw --numstat`: raw records
/// (`:old new sha sha STATUS\0path\0`, two paths for renames/copies)
/// followed by numstat records (`added\tdeleted\tpath\0`, or
/// `added\tdeleted\t\0old\0new\0` for renames; `-` for binary).
pub fn parse_diff_tree(out: &[u8]) -> Vec<CommitFile> {
    let text = String::from_utf8_lossy(out);
    let tok: Vec<&str> = text.split('\0').collect();
    let mut files: Vec<CommitFile> = Vec::new();
    let mut index: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut i = 0;
    while i < tok.len() {
        let t = tok[i];
        if t.is_empty() {
            i += 1;
        } else if let Some(raw) = t.strip_prefix(':') {
            let status = raw.rsplit(' ').next().unwrap_or("");
            let code = status.chars().next().unwrap_or('M');
            let kind = change_kind(code).unwrap_or(ChangeKind::Modified);
            let (path, orig) = if matches!(code, 'R' | 'C') {
                let r = (tok.get(i + 2), tok.get(i + 1));
                i += 3;
                (r.0.copied(), r.1.map(|s| s.to_string()))
            } else {
                let p = tok.get(i + 1).copied();
                i += 2;
                (p, None)
            };
            if let Some(p) = path {
                index.insert(p.to_string(), files.len());
                files.push(CommitFile {
                    path: p.to_string(),
                    orig_path: orig,
                    kind,
                    added: None,
                    deleted: None,
                });
            }
        } else {
            let mut parts = t.splitn(3, '\t');
            let (a, d, p) = (parts.next(), parts.next(), parts.next().unwrap_or(""));
            let path = if p.is_empty() {
                // Rename: old and new path follow as separate tokens.
                let new = tok.get(i + 2).copied();
                i += 3;
                new
            } else {
                i += 1;
                Some(p)
            };
            if let Some(f) = path.and_then(|p| index.get(p)).map(|&k| &mut files[k]) {
                f.added = a.and_then(|x| x.parse().ok());
                f.deleted = d.and_then(|x| x.parse().ok());
            }
        }
    }
    files
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
    fn diff_tree_raw_and_numstat() {
        let out = b":100644 100644 aaa bbb M\x00src/a.rs\x00:000000 100644 000 ccc A\x00new.txt\x00:100644 100644 ddd eee R087\x00old/name.rs\x00new/name.rs\x00:100644 100644 fff 000 M\x00logo.png\x003\t1\tsrc/a.rs\x005\t0\tnew.txt\x002\t2\t\x00old/name.rs\x00new/name.rs\x00-\t-\tlogo.png\x00";
        let f = parse_diff_tree(out);
        assert_eq!(f.len(), 4);
        assert_eq!(
            (f[0].path.as_str(), f[0].kind, f[0].added, f[0].deleted),
            ("src/a.rs", ChangeKind::Modified, Some(3), Some(1))
        );
        assert_eq!((f[1].kind, f[1].added), (ChangeKind::Added, Some(5)));
        assert_eq!(f[2].path, "new/name.rs");
        assert_eq!(f[2].orig_path.as_deref(), Some("old/name.rs"));
        assert_eq!((f[2].kind, f[2].added), (ChangeKind::Renamed, Some(2)));
        assert_eq!((f[3].added, f[3].deleted), (None, None));
    }

    #[test]
    fn blame_porcelain() {
        let a = "a".repeat(40);
        let z = "0".repeat(40);
        let out = format!(
            "{a} 1 1 2\nauthor Ann\nauthor-mail <ann@x>\nauthor-time 100\nauthor-tz +0000\nsummary First\nprevious {b} old.rs\nfilename new.rs\n\tfn main() {{\n{a} 2 2\n\t}}\n{z} 3 3 1\nauthor Not Committed Yet\nauthor-mail <not.committed.yet>\nauthor-time 200\nsummary Version of new.rs from new.rs\nfilename new.rs\n\t// todo",
            b = "b".repeat(40)
        );
        let b = parse_blame(out.as_bytes(), "new.rs");
        assert_eq!(b.text, "fn main() {\n}\n// todo");
        assert_eq!(b.lines, vec![0, 0, 1]);
        assert_eq!(b.commits.len(), 2);
        let c = &b.commits[0];
        assert_eq!(
            (
                c.author_name.as_str(),
                c.author_email.as_str(),
                c.author_time
            ),
            ("Ann", "ann@x", 100)
        );
        assert_eq!(c.previous_path.as_deref(), Some("old.rs"));
        assert_eq!(b.commits[1].oid, z);
    }

    #[test]
    fn stash_list() {
        let text = "stash@{0}\x00aaa\x00On main: try the parser\x001700000000\x1e\nstash@{1}\x00bbb\x00WIP on feature/x: 1a2b3c Fix bug\x001600000000\x1e\n";
        let s = parse_stash_list(text);
        assert_eq!(s.len(), 2);
        assert_eq!(
            (s[0].index, s[0].message.as_str(), s[0].branch.as_deref()),
            (0, "try the parser", Some("main"))
        );
        assert_eq!(
            (s[1].index, s[1].message.as_str(), s[1].branch.as_deref()),
            (1, "1a2b3c Fix bug", Some("feature/x"))
        );
        assert_eq!(s[1].time, 1_600_000_000);
    }

    #[test]
    fn details_message() {
        let d = parse_details(
            "abc\x00p1 p2\x00Ann\x00a@x\x00100\x00Bob\x00b@x\x00200\x00Subject\n\nBody line\n\n",
        )
        .unwrap();
        assert_eq!(d.parents, vec!["p1", "p2"]);
        assert_eq!((d.author_time, d.committer_time), (100, 200));
        assert_eq!(d.message, "Subject\n\nBody line");
    }

    #[test]
    fn progress_lines() {
        let p = parse_progress_line("Receiving objects:  45% (450/1000), 1.20 MiB | 2.00 MiB/s")
            .unwrap();
        assert_eq!(p.phase, "Receiving objects");
        assert_eq!(p.percent, Some(45));
        assert_eq!(p.detail, "(450/1000), 1.20 MiB | 2.00 MiB/s");

        let p = parse_progress_line("remote: Counting objects: 100% (10/10), done.").unwrap();
        assert_eq!(
            (p.phase.as_str(), p.percent),
            ("Counting objects", Some(100))
        );
        assert_eq!(p.detail, "(10/10)");

        let p = parse_progress_line("Enumerating objects: 5, done.").unwrap();
        assert_eq!((p.percent, p.detail.as_str()), (None, "5"));

        assert!(
            parse_progress_line("fatal: unable to access 'x': Could not resolve host").is_none()
        );
        assert!(parse_progress_line("error: failed to push some refs to 'origin'").is_none());
        assert!(parse_progress_line("To github.com:a/b.git").is_none());
        assert!(parse_progress_line(" * [new branch]      main -> main").is_none());
        assert!(parse_progress_line("hint: Updates were rejected").is_none());
    }

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
