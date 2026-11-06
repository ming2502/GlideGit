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

/// Download, verify and install the update found by the last check.
/// Returns true when the installer has taken over and the app is exiting
/// (Windows: the installer restarts the app when done); otherwise call
/// [`restart_app`] afterwards.
#[tauri::command]
pub async fn install_update(
    app: AppHandle,
    pending: State<'_, PendingUpdate>,
    state: State<'_, AppState>,
    task_id: u64,
) -> Result<bool, CmdError> {
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
    let downloaded = update
        .download(
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
    match downloaded.and_then(|bytes| install_downloaded(&app, &update, &bytes)) {
        Ok(exiting) => Ok(exiting),
        Err(e) => {
            // Keep it so "install" can be retried without checking again.
            *pending.0.lock().unwrap() = Some(update);
            Err(update_error(e))
        }
    }
}

/// Install the verified update package.
#[cfg(not(windows))]
fn install_downloaded(
    _app: &AppHandle,
    update: &Update,
    bytes: &[u8],
) -> Result<bool, tauri_plugin_updater::Error> {
    update.install(bytes).map(|()| false)
}

/// Windows: the plugin starts the NSIS installer and exits right away, and
/// the installer (after closing the app and waiting only half a second) can
/// find `glidegit.exe` still locked by the exiting process: "Error opening
/// file for writing". So the installer is started by a small hidden
/// PowerShell process that first waits until this process is gone.
#[cfg(windows)]
fn install_downloaded(
    app: &AppHandle,
    update: &Update,
    bytes: &[u8],
) -> Result<bool, tauri_plugin_updater::Error> {
    // The NSIS updater artifact is the setup program itself; anything else
    // (an MSI, a zipped bundle) goes through the plugin.
    if !bytes.starts_with(b"MZ") {
        // The plugin exits the process itself after starting the installer.
        return update.install(bytes).map(|()| true);
    }
    let path = std::env::temp_dir().join(format!("GlideGit-{}-setup.exe", update.version));
    std::fs::write(&path, bytes)?;
    windows_installer::launch_after_exit(&path)?;
    app.exit(0);
    Ok(true)
}

#[cfg(windows)]
mod windows_installer {
    use std::os::windows::process::CommandExt;
    use std::path::Path;
    use std::process::{Command, Stdio};

    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

    /// The PowerShell script: wait for `pid` to exit (at most a minute), give
    /// Windows a moment to release the executable, then run the installer
    /// passively in update mode and restart the app afterwards (`/R`).
    pub(super) fn script(pid: u32, installer: &Path) -> String {
        let path = installer.display().to_string().replace('\'', "''");
        format!(
            "Wait-Process -Id {pid} -Timeout 60 -ErrorAction SilentlyContinue; \
             Start-Sleep -Milliseconds 300; \
             Start-Process -FilePath '{path}' -ArgumentList '/P','/UPDATE','/R'"
        )
    }

    pub(super) fn launch_after_exit(installer: &Path) -> std::io::Result<()> {
        let powershell = std::env::var_os("SystemRoot")
            .map(|root| Path::new(&root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe"))
            .filter(|p| p.exists())
            .unwrap_or_else(|| "powershell.exe".into());
        Command::new(powershell)
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
            ])
            .args(["-WindowStyle", "Hidden", "-Command"])
            .arg(script(std::process::id(), installer))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .map(drop)
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn script_quotes_the_path() {
            let s = super::script(
                42,
                std::path::Path::new(r"C:\Users\O'Neil\Temp\GlideGit-1.2.0-setup.exe"),
            );
            assert!(s.starts_with("Wait-Process -Id 42 "));
            assert!(s.contains(r"-FilePath 'C:\Users\O''Neil\Temp\GlideGit-1.2.0-setup.exe'"));
            assert!(s.ends_with("-ArgumentList '/P','/UPDATE','/R'"));
        }
    }
}

/// Restart into the newly installed version.
#[tauri::command]
pub fn restart_app(app: AppHandle) {
    app.restart();
}
