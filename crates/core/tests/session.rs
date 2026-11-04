use std::fs;
use std::time::Duration;

use glide_core::{RepoSession, RepositoryManager};
use glide_git_backend::{GitCli, RunOptions};

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

fn init() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    cli()
        .run(
            dir.path(),
            &["init", "-q", "-b", "main"],
            RunOptions::write(),
        )
        .unwrap();
    dir
}

#[tokio::test(flavor = "multi_thread")]
async fn write_publishes_new_snapshot() {
    let dir = init();
    fs::write(dir.path().join("a.txt"), "a\n").unwrap();
    let session = RepoSession::open(cli(), dir.path()).await.unwrap();
    let snap = session.snapshot().unwrap();
    assert_eq!(snap.changes.len(), 1);
    assert!(snap.changes[0].staged.is_none());

    let mut rx = session.subscribe();
    rx.borrow_and_update();
    session
        .write(|r| r.stage(&["a.txt".to_string()]))
        .await
        .unwrap();
    assert!(rx.has_changed().unwrap());
    let snap = rx.borrow_and_update().clone().unwrap();
    assert!(snap.changes[0].staged.is_some());

    let oid = session.write(|r| r.commit("c1", false)).await.unwrap();
    assert_eq!(
        session.snapshot().unwrap().head.oid.as_deref(),
        Some(oid.as_str())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_write_returns_error_and_still_refreshes() {
    let dir = init();
    let session = RepoSession::open(cli(), dir.path()).await.unwrap();
    let err = session.write(|r| r.commit("nothing staged", false)).await;
    assert!(err.is_err());
    assert!(session.snapshot().is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn watcher_picks_up_external_changes() {
    let dir = init();
    let session = RepoSession::open(cli(), dir.path()).await.unwrap();
    assert!(session.snapshot().unwrap().changes.is_empty());
    let mut rx = session.subscribe();
    rx.borrow_and_update();

    // Give FSEvents/ReadDirectoryChangesW a moment to arm.
    tokio::time::sleep(Duration::from_millis(400)).await;
    fs::write(dir.path().join("external.txt"), "x\n").unwrap();

    let changed = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            rx.changed().await.unwrap();
            if let Some(s) = rx.borrow_and_update().clone() {
                if s.changes.iter().any(|c| c.path == "external.txt") {
                    return;
                }
            }
        }
    })
    .await;
    assert!(changed.is_ok(), "watcher did not report external change");
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_writes_are_serialised() {
    let dir = init();
    for i in 0..20 {
        fs::write(dir.path().join(format!("f{i}.txt")), "x").unwrap();
    }
    let session = RepoSession::open(cli(), dir.path()).await.unwrap();
    let mut handles = Vec::new();
    for i in 0..20 {
        let s = session.clone();
        handles.push(tokio::spawn(async move {
            s.write(move |r| r.stage(&[format!("f{i}.txt")])).await
        }));
    }
    for h in handles {
        h.await.unwrap().expect("no index.lock contention");
    }
    let snap = session.refresh_now().await.unwrap();
    assert!(snap.changes.iter().all(|c| c.staged.is_some()));
}

#[tokio::test(flavor = "multi_thread")]
async fn manager_dedupes_sessions() {
    let dir = init();
    fs::create_dir_all(dir.path().join("sub")).unwrap();
    let m = RepositoryManager::new(cli());
    let (a, new_a) = m.open(dir.path()).await.unwrap();
    let (b, new_b) = m.open(dir.path().join("sub")).await.unwrap();
    assert!(new_a && !new_b);
    assert!(std::sync::Arc::ptr_eq(&a, &b));
    let key = a.root().to_string_lossy().to_string();
    assert!(m.get(&key).await.is_ok());
    assert!(m.close(&key).await);
    assert!(m.get(&key).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn remote_ops_do_not_block_local_writes() {
    let dir = init();
    fs::write(dir.path().join("a.txt"), "a").unwrap();
    let session = RepoSession::open(cli(), dir.path()).await.unwrap();

    // A slow "network" op (no worktree access) holds only the network lock.
    let (started_tx, started_rx) = tokio::sync::oneshot::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let s2 = session.clone();
    let slow = tokio::spawn(async move {
        s2.remote(false, move |_| {
            let _ = started_tx.send(());
            let _ = release_rx.recv_timeout(Duration::from_secs(10));
            Ok(())
        })
        .await
    });
    started_rx.await.unwrap();

    // A local write completes while the remote op is still running.
    tokio::time::timeout(
        Duration::from_secs(5),
        session.write(|r| r.stage(&["a.txt".to_string()])),
    )
    .await
    .expect("write blocked by remote op")
    .unwrap();

    release_tx.send(()).unwrap();
    slow.await.unwrap().unwrap();
}

fn empty_commit(r: &glide_git_backend::Repo, msg: &str) -> glide_git_backend::Result<String> {
    r.cli().run(
        r.root(),
        &["commit", "-q", "--allow-empty", "-m", msg],
        RunOptions::write(),
    )?;
    let out = r
        .cli()
        .run(r.root(), &["rev-parse", "HEAD"], RunOptions::read())?;
    Ok(out.stdout_str().trim().to_string())
}

/// Wait until the index status satisfies `pred`.
async fn wait_log(session: &RepoSession, pred: impl Fn(&glide_core::LogStatus) -> bool) {
    let mut rx = session.subscribe_log().expect("session has a log index");
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if pred(&rx.borrow_and_update()) {
                return;
            }
            rx.changed().await.unwrap();
        }
    })
    .await
    .unwrap_or_else(|_| panic!("index status stuck at {:?}", session.log_status()));
}

#[tokio::test(flavor = "multi_thread")]
async fn log_index_follows_commits() {
    let dir = init();
    let cache = tempfile::tempdir().unwrap();
    let m = RepositoryManager::new(cli());
    m.set_cache_dir(cache.path().to_path_buf());
    let (session, _) = m.open(dir.path()).await.unwrap();

    // Unborn branch: an empty, complete index.
    wait_log(&session, |s| s.complete).await;
    assert_eq!(session.log_status().unwrap().total, 0);

    session.write(|r| empty_commit(r, "first")).await.unwrap();
    wait_log(&session, |s| s.complete && s.total == 1).await;
    let oid = session.write(|r| empty_commit(r, "second")).await.unwrap();
    wait_log(&session, |s| s.total == 2).await;

    let page = session.log_page(0, 50, Default::default()).await.unwrap();
    assert_eq!(page.rows[0].subject, "second");
    assert_eq!(page.rows[0].refs, vec!["HEAD -> main"]);
    assert_eq!(
        session.log_row_of(oid, Default::default()).await.unwrap(),
        Some(0)
    );

    // A session without a cache directory has no index.
    let plain = RepoSession::open(cli(), dir.path()).await.unwrap();
    assert!(plain.log_status().is_none());
    let err = plain.log_page(0, 1, Default::default()).await.unwrap_err();
    assert_eq!(err.kind(), "logIndex");
}

#[tokio::test(flavor = "multi_thread")]
async fn cached_snapshot_starts_instantly_then_refreshes() {
    let dir = init();
    let cache = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.txt"), "a\n").unwrap();

    // First open: no cache yet, the snapshot is fresh and gets written.
    {
        let m = RepositoryManager::new(cli());
        m.set_cache_dir(cache.path().to_path_buf());
        let (s, _) = m.open(dir.path()).await.unwrap();
        let snap = s.snapshot().unwrap();
        assert!(!snap.stale);
        assert_eq!(snap.changes.len(), 1);
        let files = cache.path().join("snapshots");
        tokio::time::timeout(Duration::from_secs(5), async {
            while fs::read_dir(&files).map(|d| d.count()).unwrap_or(0) == 0 {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("snapshot written to the cache");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // The repository changes while the app is closed.
    fs::write(dir.path().join("b.txt"), "b\n").unwrap();

    // Second open: the cached (stale) snapshot comes first, then the real one.
    let m = RepositoryManager::new(cli());
    m.set_cache_dir(cache.path().to_path_buf());
    let (s, _) = m.open(dir.path()).await.unwrap();
    let mut rx = s.subscribe();
    let first = rx.borrow_and_update().clone().unwrap();
    if first.stale {
        assert_eq!(first.changes.len(), 1, "cached state");
        tokio::time::timeout(Duration::from_secs(10), rx.changed())
            .await
            .unwrap()
            .unwrap();
    }
    let fresh = s.snapshot().unwrap();
    assert!(!fresh.stale);
    assert_eq!(fresh.changes.len(), 2);
}
