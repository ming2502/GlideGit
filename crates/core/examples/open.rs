//! Time from opening a repository to its first snapshot (what the UI waits
//! for), and until a fresh one is published.
//!
//! ```text
//! cargo run --release -p glide-core --example open -- <repo> <cache-dir>
//! ```
//! Run twice with the same cache directory to see a warm start.

use std::time::{Duration, Instant};

use glide_core::RepositoryManager;
use glide_git_backend::GitCli;

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let repo = args.next().expect("usage: open <repo> <cache-dir>");
    let cache = args.next().expect("usage: open <repo> <cache-dir>");
    let manager = RepositoryManager::new(GitCli::discover().unwrap());
    manager.set_cache_dir(cache.into());

    let t = Instant::now();
    let (session, _) = manager.open(&repo).await.unwrap();
    let first = session.snapshot().unwrap();
    println!(
        "first snapshot after {:?} ({} changes{})",
        t.elapsed(),
        first.changes.len(),
        if first.stale { ", from cache" } else { "" }
    );
    if first.stale {
        let mut rx = session.subscribe();
        tokio::time::timeout(Duration::from_secs(60), async {
            loop {
                rx.changed().await.unwrap();
                if rx.borrow_and_update().as_ref().is_some_and(|s| !s.stale) {
                    break;
                }
            }
        })
        .await
        .unwrap();
        println!("fresh snapshot after {:?}", t.elapsed());
    }
    // Let the cache writer finish before exiting.
    tokio::time::sleep(Duration::from_millis(300)).await;
}
