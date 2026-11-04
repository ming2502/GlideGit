//! Time snapshot refreshes (what runs after every file change).
//!
//! ```text
//! cargo run --release -p glide-git-backend --example refresh -- <repo> [rounds]
//! ```

use std::time::{Duration, Instant};

use glide_git_backend::{GitCli, Repo};

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: refresh <repo> [rounds]");
    let rounds: u32 = args.next().and_then(|n| n.parse().ok()).unwrap_or(10);
    let repo = Repo::open(GitCli::discover().unwrap(), &path).unwrap();
    let mut times = Vec::new();
    for _ in 0..rounds {
        let t = Instant::now();
        let snap = repo.snapshot().unwrap();
        times.push(t.elapsed());
        assert!(snap.root.len() > 1);
    }
    // Where the time goes.
    let t = Instant::now();
    repo.status().unwrap();
    println!("  status: {:?}", t.elapsed());
    let t = Instant::now();
    let fp = repo.refs_fingerprint();
    println!("  refs fingerprint: {:?} ({fp:x})", t.elapsed());
    let t = Instant::now();
    repo.branches().unwrap();
    println!("  branches (uncached): {:?}", t.elapsed());
    times.sort();
    let avg = times.iter().sum::<Duration>() / rounds;
    println!(
        "snapshot: median {:?}, min {:?}, avg {:?} over {rounds} rounds",
        times[times.len() / 2],
        times[0],
        avg
    );
}
