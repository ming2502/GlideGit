//! End-to-end: real git → askpass helper process → AskpassServer → handler.

use std::path::Path;
use std::sync::{Arc, Mutex};

use glide_core::askpass::{AskHandler, AskpassServer};
use glide_git_backend::{GitCli, RunOptions};

const HELPER: &str = env!("CARGO_BIN_EXE_glide-askpass");

fn isolated_cli(server: &AskpassServer) -> GitCli {
    let mut cli = GitCli::discover()
        .unwrap()
        .with_env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .with_env("GIT_CONFIG_NOSYSTEM", "1");
    for (k, v) in server.git_env(Path::new(HELPER)) {
        cli = cli.with_env(k, v);
    }
    cli
}

/// `git credential fill` with no credential helper must ask via askpass.
#[tokio::test(flavor = "multi_thread")]
async fn git_credential_prompts_go_through_handler() {
    let server = AskpassServer::bind().unwrap();
    let cli = isolated_cli(&server);
    let prompts = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = prompts.clone();
    let handler: AskHandler = Arc::new(move |prompt: String| {
        seen.lock().unwrap().push(prompt.clone());
        Box::pin(async move {
            Some(if prompt.starts_with("Username") {
                "alice".into()
            } else {
                "s3cret".into()
            })
        })
    });
    tokio::spawn(server.serve(handler));

    let dir = tempfile::tempdir().unwrap();
    let out = tokio::task::spawn_blocking(move || {
        cli.run(
            dir.path(),
            &["credential", "fill"],
            RunOptions::write().stdin(b"protocol=https\nhost=example.com\n\n".to_vec()),
        )
    })
    .await
    .unwrap()
    .unwrap();

    let text = out.stdout_str();
    assert!(text.contains("username=alice"), "{text}");
    assert!(text.contains("password=s3cret"), "{text}");
    let prompts = prompts.lock().unwrap();
    assert_eq!(prompts.len(), 2, "{prompts:?}");
    assert!(prompts[0].contains("Username for 'https://example.com'"));
    assert!(prompts[1].contains("Password for 'https://alice@example.com'"));
}

/// Cancelling in the UI (handler returns None) makes git fail instead of hang.
#[tokio::test(flavor = "multi_thread")]
async fn cancelled_prompt_fails_git() {
    let server = AskpassServer::bind().unwrap();
    let cli = isolated_cli(&server);
    let handler: AskHandler = Arc::new(|_| Box::pin(async { None }));
    tokio::spawn(server.serve(handler));

    let dir = tempfile::tempdir().unwrap();
    let res = tokio::task::spawn_blocking(move || {
        cli.run(
            dir.path(),
            &["credential", "fill"],
            RunOptions::write().stdin(b"protocol=https\nhost=example.com\n\n".to_vec()),
        )
    })
    .await
    .unwrap();
    assert!(res.is_err(), "{res:?}");
}

/// A client with the wrong token gets no answer.
#[tokio::test(flavor = "multi_thread")]
async fn wrong_token_is_rejected() {
    let server = AskpassServer::bind().unwrap();
    let addr = server.addr().to_string();
    let handler: AskHandler = Arc::new(|_| Box::pin(async { Some("leaked".into()) }));
    tokio::spawn(server.serve(handler));
    let answer = tokio::task::spawn_blocking(move || {
        glide_core::askpass::ask(&addr, "not-the-token", "Password for 'x': ")
    })
    .await
    .unwrap()
    .unwrap();
    assert_eq!(answer, None);
}
