//! In-app update (#68): the app asks GitHub Releases for `latest.json` (published only when every build of a release
//! succeeded) and, when the user clicks install, downloads the bundle, checks its signature against the public key in
//! `tauri.conf.json`, installs it and starts again. Nothing is installed without the click.
//!
//! The screens reach this only through the two commands below; the updater plugin's own JS commands are not granted
//! to the webview (no `updater:` permission in the capabilities).
use std::sync::Mutex;

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_updater::{Update, UpdaterExt};

/// The update found by the last check (kept so install takes exactly what the banner showed).
#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

#[derive(Serialize)]
pub struct Found {
    version: String,
    current: String,
}

impl From<&Update> for Found {
    fn from(u: &Update) -> Self {
        Found { version: u.version.clone(), current: u.current_version.clone() }
    }
}

fn pending(p: &Pending) -> std::sync::MutexGuard<'_, Option<Update>> {
    p.0.lock().unwrap_or_else(|e| e.into_inner())
}

/// Is a newer version published? -> null when not, or when the check failed (offline, no release yet: the failure is
/// logged, the screen shows nothing). Checked once per run while one is found; later calls answer from it.
#[tauri::command]
pub async fn update_check(app: AppHandle, state: tauri::State<'_, Pending>) -> Result<Option<Found>, String> {
    if let Some(u) = &*pending(&state) {
        return Ok(Some(u.into()));
    }
    // Windows runs the installer through `std::process::exit`, which skips the app's exit event: stop the collector
    // and the server first
    let handle = app.clone();
    let updater =
        app.updater_builder().on_before_exit(move || crate::shutdown(&handle)).build().map_err(|e| e.to_string())?;
    let found = match updater.check().await {
        Ok(found) => found,
        Err(e) => {
            life_agent_core::agent::log(&life_agent_core::config::home(), &format!("update check failed: {e}"));
            return Ok(None);
        }
    };
    let shown = found.as_ref().map(Found::from);
    *pending(&state) = found;
    Ok(shown)
}

/// Install the update the last check found, then start the new version (on Windows the installer does both).
#[tauri::command]
pub async fn update_install(app: AppHandle, state: tauri::State<'_, Pending>) -> Result<(), String> {
    let update = pending(&state).take().ok_or("설치할 새 버전이 없습니다")?;
    if let Err(e) = update.download_and_install(|_, _| {}, || {}).await {
        *pending(&state) = Some(update); // the banner stays; the user may try again
        return Err(format!("새 버전을 설치하지 못했습니다: {e}"));
    }
    crate::shutdown(&app);
    app.restart();
}
