// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod askpass_bridge;
mod commands;
mod dialogs;
mod settings;
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
    pub git: settings::GitInfo,
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

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(updater::PendingUpdate::default())
        .setup(|app| {
            // The settings file says which git to use, so git is found here,
            // once the app's directories are known.
            let config = app.path().app_config_dir().ok();
            let prefs = config.as_deref().map(settings::load).unwrap_or_default();
            let bundled = bundled_git();
            let (cli, error) = settings::resolve_git(&prefs, bundled.as_slice());
            let git = match &cli {
                Some(cli) => settings::GitInfo {
                    version: cli.version().to_string(),
                    path: cli.executable().display().to_string(),
                    bundled: bundled.iter().any(|b| b == cli.executable()),
                    problem: error,
                },
                None => settings::GitInfo {
                    version: String::new(),
                    path: String::new(),
                    bundled: false,
                    problem: error,
                },
            };
            match &git.problem {
                Some(p) => tracing::warn!("{p:?}"),
                None => tracing::info!(
                    "using git {} at {}{}",
                    git.version,
                    git.path,
                    if git.bundled { " (bundled MinGit)" } else { "" }
                ),
            }
            // Without any git the app still starts, to say so and let the
            // user point it at one; every git command fails meanwhile.
            let mut cli = cli.unwrap_or_else(|| GitCli::unchecked("git"));

            // Credential prompts are answered in the UI via our own binary as askpass.
            let bridge = Arc::new(askpass_bridge::AskpassBridge::default());
            match (AskpassServer::bind(), std::env::current_exe()) {
                (Ok(server), Ok(exe)) => {
                    for (k, v) in server.git_env(&exe) {
                        cli = cli.with_env(k, v);
                    }
                    let handler = bridge.handler();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = server.serve(handler).await {
                            tracing::error!("askpass server stopped: {e}");
                        }
                    });
                }
                (Err(e), _) | (_, Err(e)) => {
                    tracing::warn!("askpass disabled, git cannot prompt for credentials: {e}");
                }
            }
            bridge.attach(app.handle().clone());

            let manager = Arc::new(RepositoryManager::new(cli));
            match app.path().app_cache_dir() {
                Ok(dir) => manager.set_cache_dir(dir),
                Err(e) => tracing::warn!("no cache directory, Log index disabled: {e}"),
            }
            app.manage(AppState {
                manager,
                tasks: Arc::default(),
                askpass: bridge,
                git,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
            commands::set_language,
            commands::get_settings,
            commands::save_settings,
            commands::check_git,
            updater::updates_enabled,
            updater::check_update,
            updater::install_update,
            updater::restart_app,
            dialogs::center_next_dialog,
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
