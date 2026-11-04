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
    let size = std::fs::metadata(index.path())
        .map(|m| m.len())
        .unwrap_or(0);
    println!("index file: {:.1} MiB", size as f64 / 1048576.0);
}
