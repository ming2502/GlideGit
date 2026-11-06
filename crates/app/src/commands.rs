//! IPC commands exposed to the frontend. Each is a thin adapter over
//! `glide-core`; no git logic lives here.

use std::sync::Arc;

use glide_core::{
    changelists::{self, Changelists},
    AuthorCount, CoreError, LogFilter, LogPage, LogStatus, OpEntry, OpKind, OpMeta, RepoSession,
};
use glide_git_backend::{
    tr, Blame, BranchCompare, CommitDetails, CommitFile, CommitSummary, ConflictSide,
    ConflictTexts, DiffMode, FileDiff, FileRevision, FileTexts, GitCli, PullMode, PushPreview,
    RebaseStep, RepoOperation, RepoSnapshot, ResetMode, ShelfEntry, StashEntry, StashOptions,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};

use crate::settings::{self, Settings};
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

impl CmdError {
    pub fn new(kind: &'static str, message: String) -> Self {
        Self { kind, message }
    }
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

fn short(oid: &str) -> &str {
    &oid[..oid.len().min(7)]
}

/// First line of a message, shortened for an operation title.
fn headline(text: &str) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    match line.char_indices().nth(60) {
        Some((i, _)) => format!("{}…", &line[..i]),
        None => line.to_string(),
    }
}

/// "1a2b3c4 subject" for a commit, for operation titles.
async fn commit_label(s: &RepoSession, oid: &str) -> String {
    let rev = oid.to_string();
    match s.read(move |r| r.log(Some(&rev), 1)).await {
        Ok(c) if !c.is_empty() => format!("{} {}", short(oid), headline(&c[0].subject)),
        _ => short(oid).to_string(),
    }
}

/// Message of a stash entry, for undoing its drop.
async fn stash_message(s: &RepoSession, oid: &str) -> CmdResult<String> {
    let list = s.read(|r| r.stash_list()).await?;
    Ok(list
        .into_iter()
        .find(|e| e.oid == oid)
        .map(|e| e.message)
        .unwrap_or_default())
}

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
    git_path: String,
    git_bundled: bool,
    /// The configured git does not run, or there is no git at all.
    git_error: Option<String>,
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
        git_version: state.git.version.clone(),
        git_path: state.git.path.clone(),
        git_bundled: state.git.bundled,
        git_error: state.git.problem.as_ref().map(|p| p.message()),
        platform: std::env::consts::OS,
    }
}

/// The interface language, for text the backend writes ("zh" or "en").
#[tauri::command]
pub fn set_language(language: String) {
    glide_git_backend::i18n::set_english(language == "en");
}

fn config_dir(app: &AppHandle) -> CmdResult<std::path::PathBuf> {
    app.path()
        .app_config_dir()
        .map_err(|e| CmdError::new("settings", e.to_string()))
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> CmdResult<Settings> {
    Ok(settings::load(&config_dir(&app)?))
}

/// Store the settings; they take effect on the next start.
#[tauri::command]
pub fn save_settings(app: AppHandle, settings: Settings) -> CmdResult<()> {
    settings::save(&config_dir(&app)?, &settings)
        .map_err(|e| CmdError::new("settings", e.to_string()))
}

/// Run `path --version`: the version when it is a working git.
#[tauri::command]
pub async fn check_git(path: String) -> CmdResult<String> {
    tauri::async_runtime::spawn_blocking(move || GitCli::with_executable(path.trim()))
        .await
        .map_err(|e| CmdError::new("internal", e.to_string()))?
        .map(|cli| cli.version().to_string())
        .map_err(|e| CmdError::new("settings", e.to_string()))
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
    let all: Vec<String> = tracked.iter().chain(&untracked).cloned().collect();
    let title = match all.as_slice() {
        [one] => tr!("回滚 {one}", "Roll back {one}"),
        _ => tr!("回滚 {} 个文件", "Roll back {} files", all.len()),
    };
    let meta = OpMeta::new(OpKind::Rollback, title).backup_files(all);
    Ok(s.write_op(meta, move |r| r.rollback(&tracked, &untracked))
        .await?)
}

#[tauri::command]
pub async fn commit(
    state: State<'_, AppState>,
    root: String,
    message: String,
    amend: bool,
) -> CmdResult<String> {
    let s = session(&state, &root).await?;
    let meta = if amend {
        OpMeta::new(
            OpKind::Amend,
            tr!("修改提交：{}", "Amend: {}", headline(&message)),
        )
    } else {
        OpMeta::new(
            OpKind::Commit,
            tr!("提交：{}", "Commit: {}", headline(&message)),
        )
    };
    Ok(s.write_op(meta, move |r| r.commit(&message, amend)).await?)
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
pub async fn conflict_texts(
    state: State<'_, AppState>,
    root: String,
    path: String,
) -> CmdResult<ConflictTexts> {
    let s = session(&state, &root).await?;
    Ok(s.read(move |r| r.conflict_texts(&path)).await?)
}

#[tauri::command]
pub async fn resolve_conflict(
    state: State<'_, AppState>,
    root: String,
    path: String,
    content: Option<String>,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(
        s.write(move |r| r.resolve_conflict(&path, content.as_deref()))
            .await?,
    )
}

#[tauri::command]
pub async fn resolve_with_side(
    state: State<'_, AppState>,
    root: String,
    path: String,
    side: ConflictSide,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(move |r| r.resolve_with_side(&path, side)).await?)
}

#[tauri::command]
pub async fn stash_list(state: State<'_, AppState>, root: String) -> CmdResult<Vec<StashEntry>> {
    let s = session(&state, &root).await?;
    Ok(s.read(|r| r.stash_list()).await?)
}

#[tauri::command]
pub async fn stash_push(
    state: State<'_, AppState>,
    root: String,
    options: StashOptions,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    let title = match options.message.as_deref().filter(|m| !m.trim().is_empty()) {
        Some(m) => format!("Stash：{}", headline(m)),
        None => tr!("Stash 本地修改", "Stash local changes"),
    };
    let meta = OpMeta::new(OpKind::Stash, title);
    Ok(s.write_op(meta, move |r| r.stash_push(&options)).await?)
}

#[tauri::command]
pub async fn stash_apply(
    state: State<'_, AppState>,
    root: String,
    index: u32,
    oid: String,
    pop: bool,
    restore_index: bool,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    if !pop {
        return Ok(s
            .write(move |r| r.stash_apply(index, &oid, pop, restore_index))
            .await?);
    }
    let message = stash_message(&s, &oid).await?;
    let meta = OpMeta::new(
        OpKind::Stash,
        tr!("取出 stash：{}", "Pop stash: {}", headline(&message)),
    )
    .dropped_stash(oid.clone(), message);
    Ok(s.write_op(meta, move |r| {
        r.stash_apply(index, &oid, pop, restore_index)
    })
    .await?)
}

#[tauri::command]
pub async fn stash_drop(
    state: State<'_, AppState>,
    root: String,
    index: u32,
    oid: String,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    let message = stash_message(&s, &oid).await?;
    let meta = OpMeta::new(
        OpKind::Stash,
        tr!("删除 stash：{}", "Drop stash: {}", headline(&message)),
    )
    .dropped_stash(oid.clone(), message);
    Ok(s.write_op(meta, move |r| r.stash_drop(index, &oid)).await?)
}

#[tauri::command]
pub async fn stash_details(
    state: State<'_, AppState>,
    root: String,
    oid: String,
) -> CmdResult<CommitDetails> {
    let s = session(&state, &root).await?;
    Ok(s.read(move |r| r.stash_details(&oid)).await?)
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
    let meta = OpMeta::new(
        OpKind::Discard,
        tr!("丢弃 {path} 中的修改块", "Discard a block in {path}"),
    )
    .backup_files(vec![path.clone()]);
    Ok(s.write_op(meta, move |r| {
        r.write_worktree_file(&path, &content, &expected)
    })
    .await?)
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
    let meta = OpMeta::new(OpKind::Checkout, tr!("切换到 {name}", "Checkout {name}"));
    Ok(s.write_op(meta, move |r| r.checkout_branch(&name, remote))
        .await?)
}

#[tauri::command]
pub async fn checkout_detached(
    state: State<'_, AppState>,
    root: String,
    oid: String,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    let meta = OpMeta::new(
        OpKind::Checkout,
        tr!(
            "检出提交 {}",
            "Checkout commit {}",
            commit_label(&s, &oid).await
        ),
    );
    Ok(s.write_op(meta, move |r| r.checkout_detached(&oid)).await?)
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
    let meta = OpMeta::new(
        OpKind::Tag,
        tr!("新建标签 {name}（{}）", "New tag {name} ({})", short(&oid)),
    );
    Ok(
        s.write_op(meta, move |r| r.create_tag(&name, &oid, message.as_deref()))
            .await?,
    )
}

#[tauri::command]
pub async fn cherry_pick(state: State<'_, AppState>, root: String, oid: String) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    let meta = OpMeta::new(
        OpKind::CherryPick,
        format!("Cherry-pick {}", commit_label(&s, &oid).await),
    );
    Ok(s.write_op(meta, move |r| r.cherry_pick(&oid)).await?)
}

#[tauri::command]
pub async fn revert(state: State<'_, AppState>, root: String, oid: String) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    let meta = OpMeta::new(
        OpKind::Revert,
        tr!(
            "还原提交 {}",
            "Revert commit {}",
            commit_label(&s, &oid).await
        ),
    );
    Ok(s.write_op(meta, move |r| r.revert(&oid)).await?)
}

#[tauri::command]
pub async fn reset(
    state: State<'_, AppState>,
    root: String,
    oid: String,
    mode: ResetMode,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    let mode_name = match mode {
        ResetMode::Soft => "soft",
        ResetMode::Mixed => "mixed",
        ResetMode::Hard => "hard",
        ResetMode::Keep => "keep",
    };
    let title = tr!(
        "重置到 {}（{mode_name}）",
        "Reset to {} ({mode_name})",
        commit_label(&s, &oid).await
    );
    let meta = match mode {
        ResetMode::Hard => OpMeta::new(OpKind::Reset, title).backup_all(),
        _ => OpMeta::new(OpKind::Reset, title).reset_back(mode),
    };
    Ok(s.write_op(meta, move |r| r.reset(&oid, mode)).await?)
}

#[tauri::command]
pub async fn abort_operation(state: State<'_, AppState>, root: String) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(|r| r.abort_operation()).await?)
}

#[tauri::command]
pub async fn compare_branches(
    state: State<'_, AppState>,
    root: String,
    left: String,
    right: String,
) -> CmdResult<BranchCompare> {
    let s = session(&state, &root).await?;
    Ok(s.read(move |r| r.compare(&left, &right, 1000)).await?)
}

#[tauri::command]
pub async fn diff_files(
    state: State<'_, AppState>,
    root: String,
    from: String,
    to: String,
) -> CmdResult<Vec<CommitFile>> {
    let s = session(&state, &root).await?;
    Ok(s.read(move |r| r.diff_files(&from, &to)).await?)
}

#[tauri::command]
pub async fn file_history(
    state: State<'_, AppState>,
    root: String,
    path: String,
    skip: u32,
    limit: u32,
) -> CmdResult<Vec<FileRevision>> {
    let s = session(&state, &root).await?;
    Ok(s.read(move |r| r.file_history(&path, skip, limit)).await?)
}

#[tauri::command]
pub async fn line_history(
    state: State<'_, AppState>,
    root: String,
    path: String,
    start: u32,
    end: u32,
    rev: Option<String>,
) -> CmdResult<Vec<CommitSummary>> {
    let s = session(&state, &root).await?;
    Ok(
        s.read(move |r| r.line_history(&path, start, end, rev.as_deref()))
            .await?,
    )
}

#[tauri::command]
pub async fn blame(
    state: State<'_, AppState>,
    root: String,
    path: String,
    rev: Option<String>,
) -> CmdResult<Blame> {
    let s = session(&state, &root).await?;
    Ok(s.read(move |r| r.blame(&path, rev.as_deref())).await?)
}

#[tauri::command]
pub async fn rebase_plan(
    state: State<'_, AppState>,
    root: String,
    oid: String,
) -> CmdResult<Vec<CommitSummary>> {
    let s = session(&state, &root).await?;
    Ok(s.read(move |r| r.rebase_plan(&oid)).await?)
}

#[tauri::command]
pub async fn interactive_rebase(
    state: State<'_, AppState>,
    root: String,
    from: String,
    steps: Vec<RebaseStep>,
    autostash: bool,
) -> CmdResult<bool> {
    let s = session(&state, &root).await?;
    let title = tr!(
        "交互式变基 {} 个提交（自 {}）",
        "Interactive rebase of {} commits (from {})",
        steps.len(),
        short(&from)
    );
    let meta = OpMeta::new(OpKind::Rebase, title);
    Ok(s.write_op(meta, move |r| {
        r.interactive_rebase(&from, &steps, autostash)
    })
    .await?)
}

#[tauri::command]
pub async fn skip_operation(state: State<'_, AppState>, root: String) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    Ok(s.write(|r| r.skip_operation()).await?)
}

#[tauri::command]
pub async fn continue_operation(state: State<'_, AppState>, root: String) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    let what = match s.snapshot().and_then(|snap| snap.operation) {
        Some(RepoOperation::Rebase) => tr!("继续变基", "Continue rebase"),
        Some(RepoOperation::Merge) => tr!("完成合并", "Finish merge"),
        Some(RepoOperation::CherryPick) => tr!("继续 cherry-pick", "Continue cherry-pick"),
        Some(RepoOperation::Revert) => tr!("继续还原", "Continue revert"),
        None => tr!("继续操作", "Continue"),
    };
    let meta = OpMeta::new(OpKind::Continue, what);
    Ok(s.write_op(meta, |r| r.continue_operation()).await?)
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
    let title = if checkout {
        tr!("新建并切换到分支 {name}", "New branch {name}, checked out")
    } else {
        tr!("新建分支 {name}", "New branch {name}")
    };
    let meta = OpMeta::new(OpKind::Branch, title);
    Ok(s.write_op(meta, move |r| {
        r.create_branch(&name, start.as_deref(), checkout)
    })
    .await?)
}

#[tauri::command]
pub async fn delete_branch(
    state: State<'_, AppState>,
    root: String,
    name: String,
    force: bool,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    let meta = OpMeta::new(
        OpKind::Branch,
        tr!("删除分支 {name}", "Delete branch {name}"),
    );
    Ok(s.write_op(meta, move |r| r.delete_branch(&name, force))
        .await?)
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
    let (mode, title) = match mode {
        PullModeArg::Merge => (PullMode::Merge, tr!("拉取（合并）", "Pull (merge)")),
        PullModeArg::Rebase => (PullMode::Rebase, tr!("拉取（变基）", "Pull (rebase)")),
        PullModeArg::FfOnly => (
            PullMode::FastForwardOnly,
            tr!("拉取（仅快进）", "Pull (fast-forward only)"),
        ),
    };
    let task = state.tasks.start(task_id);
    let net = task.net_options(&app);
    let meta = OpMeta::new(OpKind::Pull, title);
    Ok(s.remote_op(true, meta, move |r| r.pull(mode, &net)).await?)
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
    let branch = s
        .snapshot()
        .and_then(|snap| snap.head.branch.clone())
        .unwrap_or_default();
    let title = if force_with_lease {
        tr!("强制推送 {branch}", "Force push {branch}")
    } else {
        tr!("推送 {branch}", "Push {branch}")
    };
    let meta = OpMeta::new(OpKind::Push, title);
    Ok(
        s.remote_op(false, meta, move |r| r.push(force_with_lease, &net))
            .await?,
    )
}

/// The repository's changelists.
#[tauri::command]
pub async fn changelists(state: State<'_, AppState>, root: String) -> CmdResult<Changelists> {
    let s = session(&state, &root).await?;
    Ok(s.read(|r| Ok(changelists::load(r.git_dir()))).await?)
}

/// Store the changelists; returns them as stored (repaired if needed).
#[tauri::command]
pub async fn save_changelists(
    state: State<'_, AppState>,
    root: String,
    changelists: Changelists,
) -> CmdResult<Changelists> {
    let s = session(&state, &root).await?;
    Ok(
        s.read(move |r| Ok(changelists::save(r.git_dir(), &changelists)?))
            .await?,
    )
}

#[tauri::command]
pub async fn shelf_list(state: State<'_, AppState>, root: String) -> CmdResult<Vec<ShelfEntry>> {
    let s = session(&state, &root).await?;
    Ok(s.read(|r| r.shelf_list()).await?)
}

/// Shelve the changes to `paths` and revert them.
#[tauri::command]
pub async fn shelve(
    state: State<'_, AppState>,
    root: String,
    paths: Vec<String>,
    name: String,
    changelist: Option<String>,
) -> CmdResult<ShelfEntry> {
    let s = session(&state, &root).await?;
    let meta = OpMeta::new(
        OpKind::Shelve,
        tr!("搁置：{}", "Shelve: {}", headline(&name)),
    )
    .backup_files(paths.clone());
    Ok(s.write_op(meta, move |r| {
        r.shelve(&paths, &name, changelist.as_deref())
    })
    .await?)
}

/// Apply a shelf entry; true when it left conflicts to resolve.
#[tauri::command]
pub async fn unshelve(
    state: State<'_, AppState>,
    root: String,
    id: String,
    oid: String,
    name: String,
    keep: bool,
) -> CmdResult<bool> {
    let s = session(&state, &root).await?;
    let rev = oid.clone();
    let paths = s.read(move |r| r.shelf_paths(&rev)).await?;
    let meta = OpMeta::new(
        OpKind::Shelve,
        tr!("取消搁置：{}", "Unshelve: {}", headline(&name)),
    )
    .backup_files(paths);
    Ok(s.write_op(meta, move |r| r.unshelve(&id, &oid, keep))
        .await?)
}

#[tauri::command]
pub async fn drop_shelf(
    state: State<'_, AppState>,
    root: String,
    id: String,
    oid: String,
    name: String,
) -> CmdResult<()> {
    let s = session(&state, &root).await?;
    let meta = OpMeta::new(
        OpKind::Shelve,
        tr!("删除搁置：{}", "Delete shelf entry: {}", headline(&name)),
    );
    Ok(s.write_op(meta, move |r| r.drop_shelf(&id, &oid)).await?)
}

/// Recorded operations of a repository, newest first.
#[tauri::command]
pub async fn operation_log(state: State<'_, AppState>, root: String) -> CmdResult<Vec<OpEntry>> {
    let s = session(&state, &root).await?;
    Ok(s.operations())
}

/// Undo a recorded operation. Returns what was done.
#[tauri::command]
pub async fn undo_operation(
    state: State<'_, AppState>,
    root: String,
    id: u64,
) -> CmdResult<String> {
    let s = session(&state, &root).await?;
    Ok(s.undo(id).await?)
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
