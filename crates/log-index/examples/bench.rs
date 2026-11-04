//! Time the first rows and the full build of the commit index.
//!
//! ```text
//! cargo run --release -p glide-log-index --example bench -- <repo> [cache-dir]
//! ```
//! Without `cache-dir` a fresh temporary directory is used (cold build); pass
//! the same directory twice to time a warm start.

use std::sync::Mutex;
use std::time::Instant;

use glide_git_backend::{CancelToken, GitCli, Repo};
use glide_log_index::LogIndex;

fn main() {
    let mut args = std::env::args().skip(1);
    let repo_path = args.next().expect("usage: bench <repo> [cache-dir]");
    let tmp;
    let cache = match args.next() {
        Some(d) => std::path::PathBuf::from(d),
        None => {
            tmp = std::env::temp_dir().join(format!("glidegit-bench-{}", std::process::id()));
            tmp.clone()
        }
    };

    let repo = Repo::open(GitCli::discover().unwrap(), &repo_path).unwrap();
    let index = LogIndex::open(&cache, repo.root()).unwrap();
    // Before anything is loaded: what a reopened index can show at once.
    let t = Instant::now();
    let early = index.page(0, 100).unwrap();
    if early.total > 0 {
        println!(
            "before loading: {} rows known, first page in {:?}",
            early.total,
            t.elapsed()
        );
    }

    let started = Instant::now();
    let first = Mutex::new(None);
    let kind = index
        .update(&repo, &CancelToken::new(), &|s| {
            let mut f = first.lock().unwrap();
            if f.is_none() && s.total > 0 {
                *f = Some((started.elapsed(), s.total));
            }
        })
        .unwrap();
    let total = started.elapsed();
    let status = index.status();
    if let Some((t, n)) = *first.lock().unwrap() {
        println!("first rows: {n} after {t:?}");
    }
    println!("{kind:?}: {} commits in {total:?}", status.total);

    let mid = status.total / 2;
    // The last start is a worst case: just before the next checkpoint.
    for start in [0, mid, mid + 255] {
        let t = Instant::now();
        let page = index.page(start, 100).unwrap();
        let width = page.graph.iter().map(|g| g.width()).max().unwrap_or(0);
        println!(
            "page of {} at row {start}: {:?} (graph width {width})",
            page.rows.len(),
            t.elapsed()
        );
    }
    // Filters: each one is computed fresh (the view cache holds one filter).
    use glide_log_index::LogFilter;
    let filters = [
        (
            "text",
            LogFilter {
                text: "module 42".into(),
                ..LogFilter::default()
            },
        ),
        (
            "hash",
            LogFilter {
                text: page_oid(&index),
                ..LogFilter::default()
            },
        ),
        (
            "author",
            LogFilter {
                authors: vec!["Bob".into()],
                ..LogFilter::default()
            },
        ),
        (
            "branch",
            LogFilter {
                branches: vec!["refs/heads/main".into()],
                ..LogFilter::default()
            },
        ),
        (
            "path",
            LogFilter {
                paths: vec!["README.md".into()],
                ..LogFilter::default()
            },
        ),
    ];
    for (name, f) in filters {
        let t = Instant::now();
        match index.page_filtered(&repo, &f, &CancelToken::new(), 0, 100) {
            Ok(p) => println!(
                "filter {name}: {} matches, first page in {:?}{}",
                p.total,
                t.elapsed(),
                if p.graph.is_empty() {
                    ""
                } else {
                    " (with graph)"
                }
            ),
            Err(e) => println!("filter {name}: {e}"),
        }
    }

    let size = std::fs::metadata(index.path())
        .map(|m| m.len())
        .unwrap_or(0);
    println!("index file: {:.1} MiB", size as f64 / 1048576.0);
}

fn page_oid(index: &LogIndex) -> String {
    let p = index.page(1000, 1).unwrap();
    p.rows
        .first()
        .map(|c| c.oid[..8].to_string())
        .unwrap_or_default()
}
