//! Automatic updates from GitHub releases (tauri-plugin-updater).
//!
//! Updates are signed with the project's updater key; the release workflow
//! puts the public half into the config (`plugins.updater.pubkey`, see
//! scripts/release-config.mjs). Builds without it (development, forks) simply
//! have no updater.
//!
//! The manifest (`latest.json`) lives on the latest published release, so a
//! draft release is invisible to users until it is published.

use std::sync::Mutex;

use glide_git_backend::tr;
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::commands::CmdError;
use crate::tasks::PROGRESS_EVENT;
use crate::AppState;

const ENDPOINT: &str = "https://github.com/ming2502/GlideGit/releases/latest/download/latest.json";

/// Whether the build carries an updater public key.
fn has_pubkey(app: &AppHandle) -> bool {
    app.config()
        .plugins
        .0
        .get("updater")
        .and_then(|u| u.get("pubkey"))
        .and_then(|k| k.as_str())
        .is_some_and(|k| !k.trim().is_empty())
}

/// The update found by the last check, kept until it is installed.
#[derive(Default)]
pub struct PendingUpdate(Mutex<Option<Update>>);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    current_version: String,
    /// Release notes (markdown).
    notes: Option<String>,
    /// RFC 3339.
    date: Option<String>,
}

fn update_error(e: impl std::fmt::Display) -> CmdError {
    CmdError::new("update", e.to_string())
}

/// Whether this build can update itself.
#[tauri::command]
pub fn updates_enabled(app: AppHandle) -> bool {
    has_pubkey(&app)
}

/// Ask the release server for a newer version.
#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
) -> Result<Option<UpdateInfo>, CmdError> {
    if !has_pubkey(&app) {
        return Err(CmdError::new(
            "update",
            tr!(
                "这个版本没有启用自动更新",
                "This build has no automatic updates"
            ),
        ));
    }
    // For testing against a local server; the signature check still applies.
    let endpoint = std::env::var("GLIDEGIT_UPDATE_ENDPOINT").unwrap_or_else(|_| ENDPOINT.into());
    let url = endpoint.parse().map_err(update_error)?;
    let update = app
        .updater_builder()
        .endpoints(vec![url])
        .map_err(update_error)?
        .build()
        .map_err(update_error)?
        .check()
        .await
        .map_err(update_error)?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone(),
        date: u.date.and_then(|d| {
            d.format(&time::format_description::well_known::Rfc3339)
                .ok()
        }),
    });
    *pending.0.lock().unwrap() = update;
    Ok(info)
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    phase: String,
    percent: Option<u8>,
    detail: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct ProgressEvent {
    task_id: u64,
    progress: DownloadProgress,
}

/// Download, verify and install the update found by the last check. On
/// Windows the installer takes over and the app exits; elsewhere call
/// [`restart_app`] afterwards.
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
    state: State<'_, AppState>,
    task_id: u64,
) -> Result<(), CmdError> {
    let Some(update) = pending.0.lock().unwrap().take() else {
        return Err(CmdError::new(
            "update",
            tr!(
                "没有可安装的更新，请重新检查",
                "No update to install: check again"
            ),
        ));
    };
    let _task = state.tasks.start(task_id);
    let mut received: u64 = 0;
    let mut last_percent: Option<u8> = None;
    let emitter = app.clone();
    let result = update
        .download_and_install(
            |chunk, total| {
                received += chunk as u64;
                let percent = total
                    .filter(|t| *t > 0)
                    .map(|t| (received * 100 / t).min(100) as u8);
                if percent != last_percent {
                    last_percent = percent;
                    let mb = |b: u64| format!("{:.1} MB", b as f64 / 1_048_576.0);
                    let _ = emitter.emit(
                        PROGRESS_EVENT,
                        ProgressEvent {
                            task_id,
                            progress: DownloadProgress {
                                phase: tr!("下载更新", "Downloading update"),
                                percent,
                                detail: Some(match total {
                                    Some(t) => format!("{} / {}", mb(received), mb(t)),
                                    None => mb(received),
                                }),
                            },
                        },
                    );
                }
            },
            || {
                let _ = emitter.emit(
                    PROGRESS_EVENT,
                    ProgressEvent {
                        task_id,
                        progress: DownloadProgress {
                            phase: tr!("安装更新", "Installing update"),
                            percent: None,
                            detail: None,
                        },
                    },
                );
            },
        )
        .await;
    if let Err(e) = result {
        // Keep it so "install" can be retried without checking again.
        *pending.0.lock().unwrap() = Some(update);
        return Err(update_error(e));
    }
    Ok(())
}

/// Restart into the newly installed version.
#[tauri::command]
pub fn restart_app(app: AppHandle) {
    app.restart();
}
