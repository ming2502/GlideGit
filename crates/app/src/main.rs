// Hide the console window on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod askpass_bridge;
mod commands;
mod tasks;

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
        manager: Arc::new(RepositoryManager::new(cli)),
        tasks: Arc::default(),
        askpass: bridge.clone(),
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(state)
        .setup(move |app| {
            match app.path().app_cache_dir() {
                Ok(dir) => app
                    .state::<AppState>()
                    .manager
                    .set_log_cache_dir(dir.join("log-index")),
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
            commands::checkout_branch,
            commands::create_branch,
            commands::checkout_detached,
            commands::create_tag,
            commands::cherry_pick,
            commands::revert,
            commands::reset,
            commands::abort_operation,
            commands::continue_operation,
            commands::delete_branch,
            commands::fetch,
            commands::pull,
            commands::push,
            commands::push_preview,
            commands::cancel_task,
            commands::askpass_respond,
        ])
        .run(tauri::generate_context!())
        .expect("error while running GlideGit");
}
