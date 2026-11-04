//! IPC commands exposed to the frontend. Each is a thin adapter over
//! `glide-core`; no git logic lives here.

use std::sync::Arc;

use glide_core::{AuthorCount, CoreError, LogFilter, LogPage, LogStatus, RepoSession};
use glide_git_backend::{
    CommitDetails, DiffMode, FileDiff, FileTexts, PullMode, PushPreview, RepoSnapshot, ResetMode,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::AppState;

/// Event carrying a fresh snapshot whenever a repository changes.
pub const SNAPSHOT_EVENT: &str = "repo-snapshot";
/// Event carrying the commit index status ([`LogStatus`]) as it changes.
pub const LOG_STATUS_EVENT: &str = "log-status";

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
        let snapshot_app = app.clone();
        tauri::async_runtime::spawn(async move {
            while rx.changed().await.is_ok() {
                let snap = rx.borrow_and_update().clone();
                if let Some(snapshot) = snap {
                    let _ = snapshot_app.emit(
                        SNAPSHOT_EVENT,
                        SnapshotEvent {
                            root: &root,
                            snapshot: &snapshot,
                        },
                    );
                }
            }
        });

        if let Some(mut rx) = session.subscribe_log() {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                while rx.changed().await.is_ok() {
                    let status = rx.borrow_and_update().clone();
                    let _ = app.emit(LOG_STATUS_EVENT, status);
                }
            });
        }
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
pub async fn log_status(state: State<'_, AppState>, root: String) -> CmdResult<LogStatus> {
    let s = session(&state, &root).await?;
    s.log_status().ok_or_else(|| CoreError::NoLogIndex.into())
}

#[tauri::command]
pub async fn log_page(
    state: State<'_, AppState>,
    root: String,
    start: u32,
    count: u32,
    filter: Option<LogFilter>,
) -> CmdResult<LogPage> {
    let s = session(&state, &root).await?;
    // Pages are small; a runaway request must not stall the UI.
    Ok(
        s.log_page(start, count.min(2000), filter.unwrap_or_default())
            .await?,
    )
}

#[tauri::command]
pub async fn log_row_of(
    state: State<'_, AppState>,
    root: String,
    oid: String,
    filter: Option<LogFilter>,
) -> CmdResult<Option<u32>> {
    let s = session(&state, &root).await?;
    Ok(s.log_row_of(oid, filter.unwrap_or_default()).await?)
}

#[tauri::command]
pub async fn commit_details(
    state: State<'_, AppState>,
    root: String,
    oid: String,
) -> CmdResult<CommitDetails> {
    let s = session(&state, &root).await?;
    Ok(s.read(move |r| r.commit_details(&oid)).await?)
}

#[tauri::command]
pub async fn commit_file_diff(
    state: State<'_, AppState>,
    root: String,
    oid: String,
    parent: Option<String>,
    path: String,
    orig_path: Option<String>,
) -> CmdResult<FileDiff> {
    let s = session(&state, &root).await?;
    Ok(
        s.read(move |r| r.commit_file_diff(&oid, parent.as_deref(), &path, orig_path.as_deref()))
            .await?,
    )
}

#[tauri::command]
pub async fn file_texts(
    state: State<'_, AppState>,
    root: String,
    path: String,
    orig_path: Option<String>,
    mode: DiffMode,
    untracked: bool,
) -> CmdResult<FileTexts> {
    let s = session(&state, &root).await?;
    Ok(
        s.read(move |r| r.file_texts(&path, orig_path.as_deref(), mode, untracked))
            .await?,
    )
}

#[tauri::command]
pub async fn stage_content(
    state: State<'_, AppState>,
    root: String,
    path: String,
    content: String,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(move |r| r.stage_content(&path, &content)).await?)
}

#[tauri::command]
pub async fn write_worktree_file(
    state: State<'_, AppState>,
    root: String,
    path: String,
    content: String,
    expected: String,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(
        s.write(move |r| r.write_worktree_file(&path, &content, &expected))
            .await?,
    )
}

#[tauri::command]
pub async fn commit_file_texts(
    state: State<'_, AppState>,
    root: String,
    oid: String,
    parent: Option<String>,
    path: String,
    orig_path: Option<String>,
) -> CmdResult<FileTexts> {
    let s = session(&state, &root).await?;
    Ok(
        s.read(move |r| r.commit_file_texts(&oid, parent.as_deref(), &path, orig_path.as_deref()))
            .await?,
    )
}

#[tauri::command]
pub async fn log_authors(
    state: State<'_, AppState>,
    root: String,
    query: String,
) -> CmdResult<Vec<AuthorCount>> {
    let s = session(&state, &root).await?;
    Ok(s.log_authors(query, 50).await?)
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
pub async fn checkout_detached(
    state: State<'_, AppState>,
    root: String,
    oid: String,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(move |r| r.checkout_detached(&oid)).await?)
}

#[tauri::command]
pub async fn create_tag(
    state: State<'_, AppState>,
    root: String,
    name: String,
    oid: String,
    message: Option<String>,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(
        s.write(move |r| r.create_tag(&name, &oid, message.as_deref()))
            .await?,
    )
}

#[tauri::command]
pub async fn cherry_pick(state: State<'_, AppState>, root: String, oid: String) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(move |r| r.cherry_pick(&oid)).await?)
}

#[tauri::command]
pub async fn revert(state: State<'_, AppState>, root: String, oid: String) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(move |r| r.revert(&oid)).await?)
}

#[tauri::command]
pub async fn reset(
    state: State<'_, AppState>,
    root: String,
    oid: String,
    mode: ResetMode,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(move |r| r.reset(&oid, mode)).await?)
}

#[tauri::command]
pub async fn abort_operation(state: State<'_, AppState>, root: String) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(|r| r.abort_operation()).await?)
}

#[tauri::command]
pub async fn continue_operation(state: State<'_, AppState>, root: String) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(|r| r.continue_operation()).await?)
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

/// Answer a credential prompt (`answer: null` = the user cancelled).
#[tauri::command]
pub fn askpass_respond(state: State<'_, AppState>, id: u64, answer: Option<String>) -> bool {
    state.askpass.respond(id, answer)
}
