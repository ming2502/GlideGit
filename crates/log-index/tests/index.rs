//! Log index against real repositories in temp dirs.

use std::collections::HashSet;
use std::sync::Mutex;

use glide_git_backend::{CancelToken, GitCli, Repo, RunOptions};
use glide_log_index::{LogIndex, LogStatus, UpdateKind};

fn cli() -> GitCli {
    GitCli::discover()
        .unwrap()
        .with_env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .with_env("GIT_CONFIG_NOSYSTEM", "1")
        .with_env("GIT_AUTHOR_NAME", "Test")
        .with_env("GIT_AUTHOR_EMAIL", "test@example.com")
        .with_env("GIT_COMMITTER_NAME", "Test")
        .with_env("GIT_COMMITTER_EMAIL", "test@example.com")
}

struct Fixture {
    _dir: tempfile::TempDir,
    cache: tempfile::TempDir,
    repo: Repo,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        cli()
            .run(
                dir.path(),
                &["init", "-q", "-b", "main"],
                RunOptions::write(),
            )
            .unwrap();
        let repo = Repo::open(cli(), dir.path()).unwrap();
        Self {
            _dir: dir,
            cache: tempfile::tempdir().unwrap(),
            repo,
        }
    }

    fn git(&self, args: &[&str]) -> String {
        self.repo
            .cli()
            .run(self.repo.root(), args, RunOptions::write())
            .unwrap()
            .stdout_str()
            .trim()
            .to_string()
    }

    fn commit(&self, msg: &str) -> String {
        self.git(&["commit", "-q", "--allow-empty", "-m", msg]);
        self.git(&["rev-parse", "HEAD"])
    }

    fn index(&self) -> LogIndex {
        LogIndex::open(self.cache.path(), self.repo.root()).unwrap()
    }

    /// What `git log` says should be indexed.
    fn expected(&self) -> Vec<String> {
        let out = self.git(&[
            "log",
            "--date-order",
            "--format=%H",
            "--branches",
            "--remotes",
            "--tags",
            "HEAD",
        ]);
        out.lines().map(str::to_string).collect()
    }
}

fn update(index: &LogIndex, repo: &Repo) -> UpdateKind {
    index.update(repo, &CancelToken::new(), &|_| {}).unwrap()
}

fn all_oids(index: &LogIndex) -> Vec<String> {
    let p = index.page(0, u32::MAX).unwrap();
    p.rows.into_iter().map(|c| c.oid).collect()
}

fn as_set(v: &[String]) -> HashSet<&String> {
    v.iter().collect()
}

/// Every commit appears before all of its parents.
fn assert_topological(index: &LogIndex) {
    let rows = index.page(0, u32::MAX).unwrap().rows;
    let at: std::collections::HashMap<_, _> = rows
        .iter()
        .enumerate()
        .map(|(i, c)| (c.oid.clone(), i))
        .collect();
    for (i, c) in rows.iter().enumerate() {
        for p in &c.parents {
            assert!(at[p] > i, "parent {p} of row {i} is above it");
        }
    }
}

#[test]
fn empty_repository() {
    let f = Fixture::new();
    let idx = f.index();
    assert_eq!(update(&idx, &f.repo), UpdateKind::Full);
    let s = idx.status();
    assert_eq!((s.total, s.complete, s.error), (0, true, None));
    assert_eq!(update(&idx, &f.repo), UpdateKind::Unchanged);
}

#[test]
fn full_build_matches_git_log_with_labels() {
    let f = Fixture::new();
    let root = f.commit("root");
    f.commit("second");
    f.git(&["switch", "-q", "-c", "feature"]);
    let feat = f.commit("feature work");
    f.git(&["switch", "-q", "main"]);
    f.commit("main work");
    f.git(&["merge", "-q", "--no-ff", "-m", "merge feature", "feature"]);
    f.git(&["tag", "-a", "v1", "-m", "release", &root]);

    let idx = f.index();
    assert_eq!(update(&idx, &f.repo), UpdateKind::Full);
    assert_eq!(all_oids(&idx), f.expected());
    assert_topological(&idx);

    let page = idx.page(0, 100).unwrap();
    assert_eq!(page.total, 5);
    let top = &page.rows[0];
    assert_eq!(top.subject, "merge feature");
    assert_eq!(top.refs, vec!["HEAD -> main"]);
    assert_eq!(top.parents.len(), 2);
    assert_eq!(top.author_name, "Test");
    let root_row = page.rows.iter().find(|c| c.oid == root).unwrap();
    assert_eq!(root_row.refs, vec!["tag: v1"]);
    let feat_row = page.rows.iter().find(|c| c.oid == feat).unwrap();
    assert_eq!(feat_row.refs, vec!["feature"]);
    assert!(root_row.parents.is_empty());

    assert_eq!(idx.row_of(&root).unwrap(), Some(4));
    assert_eq!(idx.row_of(&"0".repeat(40)).unwrap(), None);
    assert_eq!(idx.row_of("not hex").unwrap(), None);
}

#[test]
fn pages_are_clamped() {
    let f = Fixture::new();
    for i in 0..5 {
        f.commit(&format!("c{i}"));
    }
    let idx = f.index();
    update(&idx, &f.repo);
    let p = idx.page(3, 10).unwrap();
    assert_eq!((p.start, p.rows.len()), (3, 2));
    assert_eq!(p.rows[1].subject, "c0");
    let p = idx.page(99, 10).unwrap();
    assert_eq!((p.start, p.rows.len(), p.total), (5, 0, 5));
}

#[test]
fn new_commits_are_prepended() {
    let f = Fixture::new();
    f.commit("a");
    f.commit("b");
    let idx = f.index();
    update(&idx, &f.repo);
    let before = all_oids(&idx);
    let v = idx.status().version;

    let c = f.commit("c");
    let d = f.commit("d");
    assert_eq!(
        update(&idx, &f.repo),
        UpdateKind::Incremental {
            added: 2,
            removed: 0
        }
    );
    let after = all_oids(&idx);
    assert_eq!(after[..2], [d.clone(), c]);
    assert_eq!(after[2..], before[..]);
    assert!(idx.status().version > v);
    assert_eq!(idx.page(0, 1).unwrap().rows[0].refs, vec!["HEAD -> main"]);
    assert_eq!(idx.page(1, 1).unwrap().rows[0].refs, Vec::<String>::new());
    assert_eq!(update(&idx, &f.repo), UpdateKind::Unchanged);
}

#[test]
fn amend_and_deleted_branch_remove_rows() {
    let f = Fixture::new();
    f.commit("a");
    f.git(&["switch", "-q", "-c", "topic"]);
    let t1 = f.commit("t1");
    let t2 = f.commit("t2");
    f.git(&["switch", "-q", "main"]);
    let b = f.commit("b");
    let idx = f.index();
    update(&idx, &f.repo);
    assert_eq!(idx.status().total, 4);

    // Amend replaces b.
    f.git(&["commit", "-q", "--amend", "--allow-empty", "-m", "b2"]);
    assert_eq!(
        update(&idx, &f.repo),
        UpdateKind::Incremental {
            added: 1,
            removed: 1
        }
    );
    let oids = all_oids(&idx);
    assert!(!oids.contains(&b));
    assert_eq!(as_set(&oids), as_set(&f.expected()));

    // Deleting an unmerged branch drops its commits.
    f.git(&["branch", "-q", "-D", "topic"]);
    assert_eq!(
        update(&idx, &f.repo),
        UpdateKind::Incremental {
            added: 0,
            removed: 2
        }
    );
    let oids = all_oids(&idx);
    assert!(!oids.contains(&t1) && !oids.contains(&t2));
    assert_eq!(oids, f.expected());
    assert_topological(&idx);
    assert_eq!(idx.row_of(&t1).unwrap(), None);
}

#[test]
fn label_only_change() {
    let f = Fixture::new();
    let a = f.commit("a");
    let idx = f.index();
    update(&idx, &f.repo);
    f.git(&["branch", "other"]);
    assert_eq!(update(&idx, &f.repo), UpdateKind::Labels);
    assert_eq!(
        idx.page(0, 1).unwrap().rows[0].refs,
        vec!["HEAD -> main", "other"]
    );

    f.git(&["switch", "-q", "--detach", &a]);
    assert_eq!(update(&idx, &f.repo), UpdateKind::Labels);
    assert_eq!(
        idx.page(0, 1).unwrap().rows[0].refs,
        vec!["HEAD", "main", "other"]
    );
}

#[test]
fn reopening_reuses_the_index_on_disk() {
    let f = Fixture::new();
    f.commit("a");
    f.commit("b");
    {
        let idx = f.index();
        update(&idx, &f.repo);
    }
    let idx = f.index();
    // Not loaded yet, but the saved row count is known (M2-11).
    assert_eq!(idx.status().total, 2, "saved size before update");
    assert_eq!(update(&idx, &f.repo), UpdateKind::Labels);
    assert_eq!(all_oids(&idx), f.expected());

    // Changes made while the app was closed are picked up incrementally.
    drop(idx);
    f.commit("c");
    let idx = f.index();
    assert_eq!(
        update(&idx, &f.repo),
        UpdateKind::Incremental {
            added: 1,
            removed: 0
        }
    );
    assert_eq!(all_oids(&idx), f.expected());
}

#[test]
fn unfinished_build_is_rebuilt() {
    let f = Fixture::new();
    f.commit("a");
    let idx = f.index();
    update(&idx, &f.repo);
    let path = idx.path().to_path_buf();
    drop(idx);
    // Simulate a crash halfway through the first build.
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute("UPDATE meta SET value = '0' WHERE key = 'complete'", [])
        .unwrap();
    drop(db);
    let idx = f.index();
    assert_eq!(update(&idx, &f.repo), UpdateKind::Full);
    assert_eq!(all_oids(&idx), f.expected());
}

/// Many commits, written quickly with fast-import.
fn bulk_history(f: &Fixture, n: usize) {
    let mut s = String::new();
    for i in 1..=n {
        let msg = format!("commit {i}");
        s.push_str(&format!(
            "commit refs/heads/main\nmark :{i}\ncommitter T <t@x> {} +0000\ndata {}\n{msg}\n",
            1_600_000_000 + i,
            msg.len()
        ));
        if i > 1 {
            s.push_str(&format!("from :{}\n", i - 1));
        }
    }
    f.repo
        .cli()
        .run(
            f.repo.root(),
            &["fast-import", "--quiet"],
            RunOptions::write().stdin(s),
        )
        .unwrap();
}

#[test]
fn first_rows_are_readable_before_the_build_finishes() {
    let f = Fixture::new();
    bulk_history(&f, 7000);
    let idx = f.index();
    let seen: Mutex<Vec<LogStatus>> = Mutex::new(Vec::new());
    idx.update(&f.repo, &CancelToken::new(), &|s| {
        // Rows reported as readable really are readable, mid-build.
        if s.total > 0 {
            let p = idx.page(s.total - 1, 1).unwrap();
            assert_eq!(p.rows.len(), 1);
        }
        seen.lock().unwrap().push(s);
    })
    .unwrap();
    let seen = seen.into_inner().unwrap();
    let partial: Vec<u32> = seen
        .iter()
        .filter(|s| !s.complete && s.total > 0)
        .map(|s| s.total)
        .collect();
    // The first rows are published after the first chunk of output (>= 300
    // rows), long before all 7000 are in.
    assert!(partial.len() >= 2, "{partial:?}");
    assert!((300..2000).contains(&partial[0]), "{partial:?}");
    let last = seen.last().unwrap();
    assert_eq!((last.total, last.complete), (7000, true));
    assert_eq!(idx.page(0, 1).unwrap().rows[0].subject, "commit 7000");
    assert_eq!(idx.page(6999, 1).unwrap().rows[0].subject, "commit 1");
}

#[test]
fn cancelled_build_reports_cancelled() {
    let f = Fixture::new();
    bulk_history(&f, 3000);
    let idx = f.index();
    let cancel = CancelToken::new();
    let err = idx
        .update(&f.repo, &cancel, &|s| {
            if s.total > 0 {
                cancel.cancel();
            }
        })
        .unwrap_err();
    assert!(err.is_cancelled(), "{err}");
    assert!(!idx.status().complete);
    assert_eq!(idx.status().error, None);
    // The next update starts over and finishes.
    assert_eq!(update(&idx, &f.repo), UpdateKind::Full);
    assert_eq!(idx.status().total, 3000);
}

#[test]
fn separate_repos_get_separate_files() {
    let a = Fixture::new();
    let b = Fixture::new();
    let ia = LogIndex::open(a.cache.path(), a.repo.root()).unwrap();
    let ib = LogIndex::open(a.cache.path(), b.repo.root()).unwrap();
    assert_ne!(ia.path(), ib.path());
    assert!(ia.path().starts_with(a.cache.path()));
}

/// Graph rows computed in one pass over the whole Log, for comparison.
fn reference_graph(index: &LogIndex) -> Vec<glide_log_index::GraphRow> {
    let rows = index.page(0, u32::MAX).unwrap().rows;
    let mut layout = glide_log_index::Layout::new();
    rows.iter()
        .map(|c| {
            let oid = glide_log_index::hex_decode(&c.oid).unwrap();
            let parents: Vec<u8> = c
                .parents
                .iter()
                .flat_map(|p| glide_log_index::hex_decode(p).unwrap())
                .collect();
            let mut g = glide_log_index::GraphRow::default();
            layout.step(&oid, &parents, Some(&mut g));
            g
        })
        .collect()
}

fn assert_pages_match_reference(index: &LogIndex) {
    let reference = reference_graph(index);
    let total = reference.len() as u32;
    for start in [0, 1, 255, 256, 257, 700, total.saturating_sub(5)] {
        let page = index.page(start, 300).unwrap();
        assert_eq!(page.graph.len(), page.rows.len());
        let s = page.start as usize;
        assert_eq!(
            page.graph,
            reference[s..s + page.graph.len()],
            "page at {start}"
        );
    }
}

/// History with a long-lived side branch merged back every 40 commits.
fn branchy_history(f: &Fixture, n: usize) {
    let mut s = String::new();
    let (mut main, mut side) = (0usize, 0usize);
    for i in 1..=n {
        let on_side = i % 40 >= 25 && i % 40 < 35;
        let msg = format!("c{i}");
        let branch = if on_side { "side" } else { "main" };
        s.push_str(&format!(
            "commit refs/heads/{branch}\nmark :{i}\ncommitter T <t@x> {} +0000\ndata {}\n{msg}\n",
            1_600_000_000 + i,
            msg.len()
        ));
        if on_side {
            let from = if side == 0 || i % 40 == 25 {
                main
            } else {
                side
            };
            if from > 0 {
                s.push_str(&format!("from :{from}\n"));
            }
            side = i;
        } else {
            if main > 0 {
                s.push_str(&format!("from :{main}\n"));
            }
            if i % 40 == 35 && side > 0 {
                s.push_str(&format!("merge :{side}\n"));
            }
            main = i;
        }
    }
    f.repo
        .cli()
        .run(
            f.repo.root(),
            &["fast-import", "--quiet"],
            RunOptions::write().stdin(s),
        )
        .unwrap();
    f.git(&["reset", "-q", "--hard", "main"]);
}

#[test]
fn graph_pages_match_a_full_layout() {
    let f = Fixture::new();
    branchy_history(&f, 1500);
    let idx = f.index();
    update(&idx, &f.repo);
    let widths: Vec<u16> = idx
        .page(0, 100)
        .unwrap()
        .graph
        .iter()
        .map(|g| g.width())
        .collect();
    assert!(widths.iter().any(|&w| w >= 2), "{widths:?}");
    assert_pages_match_reference(&idx);

    // Checkpoints rebuilt from disk give the same result.
    drop(idx);
    let idx = f.index();
    update(&idx, &f.repo);
    assert_pages_match_reference(&idx);

    // New commits on main and on a new branch, then an amend, then a branch
    // deletion: every incremental update keeps pages equal to a fresh layout.
    f.commit("on main");
    f.git(&["switch", "-q", "-c", "topic", "HEAD~30"]);
    f.commit("topic 1");
    f.commit("topic 2");
    assert!(matches!(
        update(&idx, &f.repo),
        UpdateKind::Incremental { added: 3, .. }
    ));
    assert_pages_match_reference(&idx);

    f.git(&["commit", "-q", "--amend", "--allow-empty", "-m", "topic 2b"]);
    update(&idx, &f.repo);
    assert_pages_match_reference(&idx);

    f.git(&["switch", "-q", "main"]);
    f.git(&["branch", "-q", "-D", "topic", "side"]);
    assert!(matches!(
        update(&idx, &f.repo),
        UpdateKind::Incremental { removed: 2, .. }
    ));
    assert_pages_match_reference(&idx);
}

fn filtered(idx: &LogIndex, f: &Fixture, filter: &glide_log_index::LogFilter) -> Vec<String> {
    idx.page_filtered(&f.repo, filter, &CancelToken::new(), 0, 10_000)
        .unwrap()
        .rows
        .into_iter()
        .map(|c| c.subject)
        .collect()
}

#[test]
fn filters() {
    use glide_log_index::LogFilter;
    let f = Fixture::new();
    let write = |path: &str, text: &str| {
        let p = f.repo.root().join(path);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
        f.git(&["add", "-A"]);
    };
    write("src/a.rs", "1");
    f.git(&[
        "commit",
        "-q",
        "-m",
        "Add parser",
        "--date=@1000000000 +0000",
    ]);
    write("docs/readme.md", "1");
    f.git(&[
        "-c",
        "user.name=Alice",
        "-c",
        "user.email=alice@corp.io",
        "commit",
        "-q",
        "-m",
        "Write 100% of the docs",
        "--author=Alice <alice@corp.io>",
        "--date=@1100000000 +0000",
    ]);
    f.git(&["switch", "-q", "-c", "topic"]);
    write("src/a.rs", "2");
    let topic = f.commit("Fix parser bug");
    f.git(&["switch", "-q", "main"]);
    write("src/b.rs", "1");
    f.git(&["commit", "-q", "-m", "Add b", "--date=@1300000000 +0000"]);

    let idx = f.index();
    update(&idx, &f.repo);
    let all = filtered(&idx, &f, &LogFilter::default());
    assert_eq!(all.len(), 5 - 1, "{all:?}");

    let text = |t: &str| LogFilter {
        text: t.into(),
        ..LogFilter::default()
    };
    assert_eq!(
        filtered(&idx, &f, &text("PARSER")),
        ["Fix parser bug", "Add parser"]
    );
    // LIKE wildcards are literal.
    assert_eq!(
        filtered(&idx, &f, &text("100%")),
        ["Write 100% of the docs"]
    );
    assert!(filtered(&idx, &f, &text("_")).is_empty());
    assert_eq!(filtered(&idx, &f, &text(&topic[..7])), ["Fix parser bug"]);

    let authors = LogFilter {
        authors: vec!["corp.io".into()],
        ..LogFilter::default()
    };
    assert_eq!(filtered(&idx, &f, &authors), ["Write 100% of the docs"]);

    let dates = LogFilter {
        since: Some(1_050_000_000),
        until: Some(1_200_000_000),
        ..LogFilter::default()
    };
    assert_eq!(filtered(&idx, &f, &dates), ["Write 100% of the docs"]);

    let paths = LogFilter {
        paths: vec!["src/a.rs".into()],
        ..LogFilter::default()
    };
    let page = idx
        .page_filtered(&f.repo, &paths, &CancelToken::new(), 0, 100)
        .unwrap();
    assert_eq!(page.rows.len(), 2);
    assert!(page.graph.is_empty(), "path filters have no graph");

    // Branch filter: only history reachable from `topic`, with a graph.
    let branch = LogFilter {
        branches: vec!["refs/heads/topic".into()],
        ..LogFilter::default()
    };
    let page = idx
        .page_filtered(&f.repo, &branch, &CancelToken::new(), 0, 100)
        .unwrap();
    let subjects: Vec<_> = page.rows.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(
        subjects,
        ["Fix parser bug", "Write 100% of the docs", "Add parser"]
    );
    assert_eq!(page.graph.len(), 3);
    assert!(page.graph.iter().all(|g| g.lane == 0));
    assert_eq!(
        idx.row_of_filtered(&f.repo, &branch, &CancelToken::new(), &topic)
            .unwrap(),
        Some(0)
    );

    // Branch + path combine.
    let both = LogFilter {
        branches: vec!["refs/heads/main".into()],
        paths: vec!["src".into()],
        ..LogFilter::default()
    };
    assert_eq!(filtered(&idx, &f, &both), ["Add b", "Add parser"]);

    let bad = LogFilter {
        branches: vec!["--all".into()],
        ..LogFilter::default()
    };
    assert!(idx
        .page_filtered(&f.repo, &bad, &CancelToken::new(), 0, 10)
        .is_err());

    // Suggestions, most commits first.
    let a = idx.authors("", 10).unwrap();
    assert_eq!(a[0].name, "Test");
    assert_eq!(a[0].commits, 3);
    assert_eq!(idx.authors("alice", 10).unwrap().len(), 1);

    // A view follows new commits once the index has them.
    f.commit("Parser docs");
    update(&idx, &f.repo);
    assert_eq!(filtered(&idx, &f, &text("parser"))[0], "Parser docs");
}

#[test]
fn branch_filter_graph_matches_a_layout_of_the_subset() {
    use glide_log_index::LogFilter;
    let f = Fixture::new();
    branchy_history(&f, 900);
    let idx = f.index();
    update(&idx, &f.repo);
    let filter = LogFilter {
        branches: vec!["refs/heads/side".into()],
        ..LogFilter::default()
    };
    let all = idx
        .page_filtered(&f.repo, &filter, &CancelToken::new(), 0, u32::MAX)
        .unwrap();
    assert!(all.total > 300 && all.total < 900, "{}", all.total);
    let mut layout = glide_log_index::Layout::new();
    let reference: Vec<_> = all
        .rows
        .iter()
        .map(|c| {
            let oid = glide_log_index::hex_decode(&c.oid).unwrap();
            let parents: Vec<u8> = c
                .parents
                .iter()
                .flat_map(|p| glide_log_index::hex_decode(p).unwrap())
                .collect();
            let mut g = glide_log_index::GraphRow::default();
            layout.step(&oid, &parents, Some(&mut g));
            g
        })
        .collect();
    for start in [0u32, 255, 256, 300] {
        let p = idx
            .page_filtered(&f.repo, &filter, &CancelToken::new(), start, 100)
            .unwrap();
        let s = p.start as usize;
        assert_eq!(p.graph, reference[s..s + p.graph.len()], "page at {start}");
    }
}

#[test]
fn first_pages_are_served_before_the_index_is_loaded() {
    let f = Fixture::new();
    branchy_history(&f, 700);
    f.git(&["tag", "v1", "HEAD~3"]);
    {
        let idx = f.index();
        update(&idx, &f.repo);
    }
    let idx = f.index();
    // Nothing loaded yet, but the saved size is known.
    let s = idx.status();
    assert_eq!((s.total, s.complete), (700, true));
    let early: Vec<_> = [0u32, 250, 690]
        .iter()
        .map(|&st| idx.page(st, 100).unwrap())
        .collect();
    // Rows can be found before loading too.
    let oid = early[1].rows[7].oid.clone();
    assert_eq!(idx.row_of(&oid).unwrap(), Some(257));
    assert!(
        early[0].rows[3].refs.contains(&"tag: v1".to_string()),
        "labels saved too"
    );

    update(&idx, &f.repo);
    assert!(
        idx.status().version > s.version,
        "rows are re-read after loading"
    );
    for (p, &start) in early.iter().zip(&[0u32, 250, 690]) {
        let loaded = idx.page(start, 100).unwrap();
        assert_eq!(p.rows, loaded.rows, "rows at {start}");
        assert_eq!(p.graph, loaded.graph, "graph at {start}");
    }
}
