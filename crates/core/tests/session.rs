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
