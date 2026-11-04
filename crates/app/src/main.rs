// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod tasks;

use std::sync::Arc;

use glide_core::RepositoryManager;
use glide_git_backend::GitCli;

pub struct AppState {
    pub manager: Arc<RepositoryManager>,
    pub tasks: Arc<tasks::TaskRegistry>,
    pub git_version: String,
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = match GitCli::discover() {
        Ok(cli) => cli,
        Err(e) => {
            // TODO(M4-5): show a "git not found" setup screen instead of exiting.
            tracing::error!("{e}");
            GitCli::with_executable("git").unwrap_or_else(|_| panic!("git not found: {e}"))
        }
    };
    tracing::info!(
        "using git {} at {}",
        cli.version(),
        cli.executable().display()
    );

    let state = AppState {
        git_version: cli.version().to_string(),
        manager: Arc::new(RepositoryManager::new(cli)),
        tasks: Arc::default(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            commands::app_info,
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
            commands::log,
            commands::checkout_branch,
            commands::create_branch,
            commands::delete_branch,
            commands::fetch,
            commands::pull,
            commands::push,
            commands::push_preview,
            commands::cancel_task,
        ])
        .run(tauri::generate_context!())
        .expect("error while running GlideGit");
}
