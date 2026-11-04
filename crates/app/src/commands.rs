//! IPC commands exposed to the frontend. Each is a thin adapter over
//! `glide-core`; no git logic lives here.

use std::sync::Arc;

use glide_core::{CoreError, RepoSession};
use glide_git_backend::{CommitSummary, DiffMode, FileDiff, PullMode, PushPreview, RepoSnapshot};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::AppState;

/// Event carrying a fresh snapshot whenever a repository changes.
pub const SNAPSHOT_EVENT: &str = "repo-snapshot";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CmdError {
    kind: &'static str,
    message: String,
}

impl From<CoreError> for CmdError {
    fn from(e: CoreError) -> Self {
        Self {
            kind: e.kind(),
            message: e.to_string(),
        }
    }
}

type CmdResult<T> = Result<T, CmdError>;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct SnapshotEvent<'a> {
    root: &'a str,
    snapshot: &'a RepoSnapshot,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    version: &'static str,
    git_version: String,
    platform: &'static str,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PullModeArg {
    Merge,
    Rebase,
    FfOnly,
}

async fn session(state: &State<'_, AppState>, root: &str) -> CmdResult<Arc<RepoSession>> {
    Ok(state.manager.get(root).await?)
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION"),
        git_version: state.git_version.clone(),
        platform: std::env::consts::OS,
    }
}

#[tauri::command]
pub async fn open_repo(
    app: AppHandle,
    state: State<'_, AppState>,
    path: String,
) -> CmdResult<RepoSnapshot> {
    let (session, newly_opened) = state.manager.open(&path).await?;
    let root = session.root().to_string_lossy().into_owned();

    if newly_opened {
        // Forward every published snapshot to the UI until the session is dropped.
        let mut rx = session.subscribe();
        rx.borrow_and_update();
        let root = root.clone();
        tauri::async_runtime::spawn(async move {
            while rx.changed().await.is_ok() {
                let snap = rx.borrow_and_update().clone();
                if let Some(snapshot) = snap {
                    let _ = app.emit(
                        SNAPSHOT_EVENT,
                        SnapshotEvent {
                            root: &root,
                            snapshot: &snapshot,
                        },
                    );
                }
            }
        });
    }

    match session.snapshot() {
        Some(s) => Ok((*s).clone()),
        None => Ok((*session.refresh_now().await?).clone()),
    }
}

#[tauri::command]
pub async fn close_repo(state: State<'_, AppState>, root: String) -> CmdResult<bool> {
    Ok(state.manager.close(&root).await)
}

#[tauri::command]
pub async fn refresh(state: State<'_, AppState>, root: String) -> CmdResult<RepoSnapshot> {
    let s = session(&state, &root).await?;
    Ok((*s.refresh_now().await?).clone())
}

#[tauri::command]
pub async fn stage(state: State<'_, AppState>, root: String, paths: Vec<String>) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(move |r| r.stage(&paths)).await?)
}

#[tauri::command]
pub async fn unstage(
    state: State<'_, AppState>,
    root: String,
    paths: Vec<String>,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(move |r| r.unstage(&paths)).await?)
}

#[tauri::command]
pub async fn stage_all(state: State<'_, AppState>, root: String) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(|r| r.stage_all()).await?)
}

#[tauri::command]
pub async fn rollback(
    state: State<'_, AppState>,
    root: String,
    tracked: Vec<String>,
    untracked: Vec<String>,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(move |r| r.rollback(&tracked, &untracked)).await?)
}

#[tauri::command]
pub async fn commit(
    state: State<'_, AppState>,
    root: String,
    message: String,
    amend: bool,
) -> CmdResult<String> {
    let s = session(&state, &root).await?;
    Ok(s.write(move |r| r.commit(&message, amend)).await?)
}

#[tauri::command]
pub async fn last_commit_message(state: State<'_, AppState>, root: String) -> CmdResult<String> {
    let s = session(&state, &root).await?;
    Ok(s.read(|r| r.last_commit_message()).await?)
}

#[tauri::command]
pub async fn diff_file(
    state: State<'_, AppState>,
    root: String,
    path: String,
    mode: DiffMode,
    untracked: bool,
) -> CmdResult<FileDiff> {
    let s = session(&state, &root).await?;
    Ok(s.read(move |r| r.diff_file(&path, mode, untracked)).await?)
}

#[tauri::command]
pub async fn log(
    state: State<'_, AppState>,
    root: String,
    limit: usize,
) -> CmdResult<Vec<CommitSummary>> {
    let s = session(&state, &root).await?;
    Ok(s.read(move |r| r.log(None, limit)).await?)
}

#[tauri::command]
pub async fn checkout_branch(
    state: State<'_, AppState>,
    root: String,
    name: String,
    remote: bool,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(move |r| r.checkout_branch(&name, remote)).await?)
}

#[tauri::command]
pub async fn create_branch(
    state: State<'_, AppState>,
    root: String,
    name: String,
    start: Option<String>,
    checkout: bool,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(
        s.write(move |r| r.create_branch(&name, start.as_deref(), checkout))
            .await?,
    )
}

#[tauri::command]
pub async fn delete_branch(
    state: State<'_, AppState>,
    root: String,
    name: String,
    force: bool,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(move |r| r.delete_branch(&name, force)).await?)
}

#[tauri::command]
pub async fn fetch(
    app: AppHandle,
    state: State<'_, AppState>,
    root: String,
    task_id: u64,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    let task = state.tasks.start(task_id);
    let net = task.net_options(&app);
    Ok(s.remote(false, move |r| r.fetch(&net)).await?)
}

#[tauri::command]
pub async fn pull(
    app: AppHandle,
    state: State<'_, AppState>,
    root: String,
    mode: PullModeArg,
    task_id: u64,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    let mode = match mode {
        PullModeArg::Merge => PullMode::Merge,
        PullModeArg::Rebase => PullMode::Rebase,
        PullModeArg::FfOnly => PullMode::FastForwardOnly,
    };
    let task = state.tasks.start(task_id);
    let net = task.net_options(&app);
    Ok(s.remote(true, move |r| r.pull(mode, &net)).await?)
}

#[tauri::command]
pub async fn push(
    app: AppHandle,
    state: State<'_, AppState>,
    root: String,
    force_with_lease: bool,
    task_id: u64,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    let task = state.tasks.start(task_id);
    let net = task.net_options(&app);
    Ok(s.remote(false, move |r| r.push(force_with_lease, &net))
        .await?)
}

#[tauri::command]
pub async fn push_preview(state: State<'_, AppState>, root: String) -> CmdResult<PushPreview> {
    let s = session(&state, &root).await?;
    Ok(s.read(|r| r.push_preview(500)).await?)
}

/// Request cancellation of a running task. Returns false if it already finished.
#[tauri::command]
pub fn cancel_task(state: State<'_, AppState>, task_id: u64) -> bool {
    state.tasks.cancel(task_id)
}
