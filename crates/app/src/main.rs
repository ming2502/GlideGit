// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod askpass_bridge;
mod commands;
mod tasks;
mod updater;

use std::sync::Arc;

use glide_core::askpass::{self, AskpassServer};
use glide_core::RepositoryManager;
use glide_git_backend::GitCli;
use tauri::Manager;

pub struct AppState {
    pub manager: Arc<RepositoryManager>,
    pub tasks: Arc<tasks::TaskRegistry>,
    pub askpass: Arc<askpass_bridge::AskpassBridge>,
    pub git_version: String,
    pub git_path: String,
    /// Using the MinGit shipped with the app (no other git was found).
    pub git_bundled: bool,
}

fn main() {
    // Started by git/ssh as GIT_ASKPASS / SSH_ASKPASS: answer the prompt and exit.
    if let Some(code) = askpass::client_main_if_requested() {
        std::process::exit(code);
    }

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let bundled = bundled_git();
    let cli = match GitCli::discover_with(bundled.as_slice()) {
        Ok(cli) => cli,
        Err(e) => {
            // TODO(M4-5): show a "git not found" setup screen instead of exiting.
            tracing::error!("{e}");
            GitCli::with_executable("git").unwrap_or_else(|_| panic!("git not found: {e}"))
        }
    };
    let git_bundled = bundled.iter().any(|b| b == cli.executable());
    tracing::info!(
        "using git {} at {}{}",
        cli.version(),
        cli.executable().display(),
        if git_bundled { " (bundled MinGit)" } else { "" }
    );

    // Credential prompts are answered in the UI via our own binary as askpass.
    let mut cli = cli;
    let askpass_server = match (AskpassServer::bind(), std::env::current_exe()) {
        (Ok(server), Ok(exe)) => {
            for (k, v) in server.git_env(&exe) {
                cli = cli.with_env(k, v);
            }
            Some(server)
        }
        (Err(e), _) | (_, Err(e)) => {
            tracing::warn!("askpass disabled, git cannot prompt for credentials: {e}");
            None
        }
    };
    let bridge = Arc::new(askpass_bridge::AskpassBridge::default());

    let state = AppState {
        git_version: cli.version().to_string(),
        git_path: cli.executable().display().to_string(),
        git_bundled,
        manager: Arc::new(RepositoryManager::new(cli)),
        tasks: Arc::default(),
        askpass: bridge.clone(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(state)
        .manage(updater::PendingUpdate::default())
        .setup(move |app| {
            match app.path().app_cache_dir() {
                Ok(dir) => app.state::<AppState>().manager.set_cache_dir(dir),
                Err(e) => tracing::warn!("no cache directory, Log index disabled: {e}"),
            }
            bridge.attach(app.handle().clone());
            if let Some(server) = askpass_server {
                let handler = bridge.handler();
                tauri::async_runtime::spawn(async move {
                    if let Err(e) = server.serve(handler).await {
                        tracing::error!("askpass server stopped: {e}");
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            updater::updates_enabled,
            updater::check_update,
            updater::install_update,
            updater::restart_app,
            commands::open_repo,
            commands::close_repo,
            commands::refresh,
            commands::stage,
            commands::unstage,
            commands::stage_all,
            commands::rollback,
            commands::commit,
            commands::last_commit_message,
            commands::diff_file,
            commands::log_status,
            commands::log_page,
            commands::log_row_of,
            commands::log_authors,
            commands::commit_details,
            commands::commit_file_diff,
            commands::file_texts,
            commands::commit_file_texts,
            commands::stage_content,
            commands::conflict_texts,
            commands::resolve_conflict,
            commands::resolve_with_side,
            commands::stash_list,
            commands::stash_push,
            commands::stash_apply,
            commands::stash_drop,
            commands::stash_details,
            commands::write_worktree_file,
            commands::checkout_branch,
            commands::create_branch,
            commands::checkout_detached,
            commands::create_tag,
            commands::cherry_pick,
            commands::revert,
            commands::reset,
            commands::abort_operation,
            commands::continue_operation,
            commands::skip_operation,
            commands::rebase_plan,
            commands::blame,
            commands::file_history,
            commands::compare_branches,
            commands::diff_files,
            commands::line_history,
            commands::interactive_rebase,
            commands::delete_branch,
            commands::fetch,
            commands::pull,
            commands::push,
            commands::push_preview,
            commands::operation_log,
            commands::undo_operation,
            commands::changelists,
            commands::save_changelists,
            commands::shelf_list,
            commands::shelve,
            commands::unshelve,
            commands::drop_shelf,
            commands::cancel_task,
            commands::askpass_respond,
        ])
        .run(tauri::generate_context!())
        .expect("error while running GlideGit");
}

/// The git shipped with the app, if any: MinGit, installed next to the
/// executable on Windows (see `tauri.windows.conf.json`). Used only when no
/// other git is found.
fn bundled_git() -> Option<std::path::PathBuf> {
    if !cfg!(windows) {
        return None;
    }
    let dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    Some(dir.join("mingit").join("cmd").join("git.exe"))
}
