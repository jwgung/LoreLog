//! LoreLog desktop shell (Tauri).
//!
//! Two jobs, both thin over `life_agent_core`:
//!   1. **The screens** — the window loads the app's own bundled UI (`index.html` = the assistant: chat, to-dos,
//!      drawer; `settings.html` = server address, token, watched folders). The assistant UI talks to the server's
//!      API over the network, using the address and token it reads back through `server_config`. The screens are
//!      bundled in the app, not served to browsers.
//!   2. **The PC collector** — a background thread runs the same loop as `life-agent run`: window segments plus the
//!      watched-folder intake (deliverables the user drops into folders they chose). The settings the loop reads
//!      (url, token, watched folders) live in the core config, shared with the CLI.
//!
//! The shell owns no judgement about the diary: it exposes the config to the UI as commands and runs the collector.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use life_agent_core::config;
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};

#[derive(Serialize)]
struct Settings {
    url: String,
    /// Never send the token back to the screen; only whether it is set.
    has_token: bool,
    watch_dirs: Vec<String>,
    config_path: String,
}

#[derive(Serialize)]
struct ServerConfig {
    base: String,
    token: String,
}

#[tauri::command]
fn get_settings() -> Result<Settings, String> {
    let cfg = config::load()?;
    Ok(Settings {
        has_token: !cfg.token.is_empty(),
        config_path: config::config_path().to_string_lossy().to_string(),
        url: cfg.url,
        watch_dirs: cfg.watch_dirs,
    })
}

/// The server address and token the bundled assistant UI needs to call the API. Returned only to the app's own
/// screen (a Tauri command), never exposed as a web endpoint.
#[tauri::command]
fn server_config() -> Result<ServerConfig, String> {
    let cfg = config::load()?;
    Ok(ServerConfig {
        base: cfg.url.trim_end_matches('/').to_string(),
        token: cfg.token,
    })
}

/// Save the server address and (if given) the token. An empty token leaves the stored one untouched. The address
/// must be https, or http only to a loopback/tailnet host (the token would otherwise travel in clear).
#[tauri::command]
fn save_settings(url: String, token: Option<String>) -> Result<(), String> {
    let url = url.trim().trim_end_matches('/').to_string();
    if !acceptable_url(&url) {
        return Err("주소는 https여야 합니다 (http는 로컬/테일넷만)".into());
    }
    let mut cfg = config::load()?;
    cfg.url = url;
    if let Some(t) = token {
        let t = t.trim();
        if !t.is_empty() {
            cfg.token = t.to_string();
        }
    }
    config::save(&cfg)
}

/// The address must be a tailnet host (the server lives on the tailnet). https to a `*.ts.net` host, or http only to
/// loopback (`localhost`/`127.0.0.1`/`::1`) or `*.ts.net`. Nothing else — a mistyped public host must not receive the
/// Bearer token. Parsed with `url` so userinfo (`user@host`), ports and IPv6 brackets are handled the way the real
/// HTTP client parses them (a bare `host.split(':')` would let `http://127.0.0.1@evil.com` through).
fn acceptable_url(url: &str) -> bool {
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    let Some(host) = parsed.host_str() else {
        return false;
    };
    match parsed.scheme() {
        "https" => is_tailnet(host),
        "http" => is_loopback(host) || is_tailnet(host),
        _ => false,
    }
}

fn is_tailnet(host: &str) -> bool {
    host == "ts.net" || host.ends_with(".ts.net")
}

fn is_loopback(host: &str) -> bool {
    // url's host_str keeps the IPv6 brackets, so ::1 arrives as "[::1]"
    host == "localhost" || host == "127.0.0.1" || host == "::1" || host == "[::1]"
}

/// Add a folder to watch (the real absolute path is stored so the intake journal keys stay stable).
#[tauri::command]
fn add_watch(dir: String) -> Result<Vec<String>, String> {
    let mut cfg = config::load()?;
    config::add_watch_dir(&mut cfg, &dir)
}

#[tauri::command]
fn remove_watch(dir: String) -> Result<Vec<String>, String> {
    let mut cfg = config::load()?;
    config::remove_watch_dir(&mut cfg, &dir).map(|(dirs, _)| dirs)
}

/// Navigate the main window to one of the app's bundled pages (the assistant or the settings).
fn go(app: &AppHandle, page: &str) {
    // the bundled frontend is served from the custom protocol: tauri://localhost (Linux/macOS) or http://tauri.localhost (Windows)
    let origin = if cfg!(windows) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    };
    if let Some(w) = app.get_webview_window("main")
        && let Ok(url) = format!("{origin}/{page}").parse()
    {
        let _ = w.navigate(url);
    }
}

/// The collection loop, run on a background thread for as long as the app lives. It is exactly `life-agent run`'s
/// loop (`run_until`), driven by the app's stop flag instead of Ctrl+C. `run_until` returns an error until url and
/// token are set, so we retry; a repeated error is logged once so a real problem is not silent. Note the shell does
/// not take the CLI's single-instance lock, so do not also run `life-agent run` at the same time.
fn start_collector(stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut last_err = String::new();
        loop {
            if stop.load(Ordering::SeqCst) {
                return;
            }
            match life_agent::run::run_until(stop.clone()) {
                Ok(()) => return, // stopped cleanly (app exiting)
                Err(e) => {
                    if e != last_err {
                        life_agent_core::agent::log(&config::home(), &format!("collector not running: {e}"));
                        last_err = e;
                    }
                    std::thread::sleep(Duration::from_secs(5)); // not configured yet, or a transient problem
                }
            }
        }
    });
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "열기", true, None::<&str>)?;
    let chat = MenuItem::with_id(app, "chat", "채팅 열기", true, None::<&str>)?;
    let settings = MenuItem::with_id(app, "settings", "설정", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "종료", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &chat, &settings, &quit])?;
    TrayIconBuilder::new()
        .icon(app.default_window_icon().unwrap().clone())
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "chat" => {
                show_main(app);
                go(app, "index.html");
            }
            "settings" => {
                show_main(app);
                go(app, "settings.html");
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { .. } = event {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_exit = stop.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }))
        .invoke_handler(tauri::generate_handler![
            get_settings,
            server_config,
            save_settings,
            add_watch,
            remove_watch
        ])
        .setup(move |app| {
            build_tray(app.handle())?;
            // the window loads index.html (the assistant), which reads server_config and shows setup or chat itself
            start_collector(stop.clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            // closing the window hides it; the collector keeps running in the tray
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building the app")
        .run(move |_app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                stop_for_exit.store(true, Ordering::SeqCst);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::acceptable_url;

    #[test]
    fn only_tailnet_or_loopback_addresses_are_accepted() {
        // the server lives on the tailnet; a public host must never receive the token
        assert!(acceptable_url("https://gungbuntu.tail8d1ab7.ts.net:48765"));
        assert!(acceptable_url("http://localhost:8765"));
        assert!(acceptable_url("http://127.0.0.1:8765"));
        assert!(acceptable_url("http://[::1]:8765"));
        assert!(acceptable_url("http://host.tail8d1ab7.ts.net:8765"));

        assert!(!acceptable_url("https://evil.example.com"), "a public https host is refused");
        assert!(!acceptable_url("http://evil.example.com"), "http off-tailnet is refused");
        assert!(!acceptable_url("https://ts.net.evil.com"), "a suffix lookalike is refused");
        assert!(!acceptable_url(""), "empty is refused");
        assert!(!acceptable_url("ftp://x.ts.net"), "a non-http scheme is refused");
        assert!(!acceptable_url("http://8.8.8.8:8765"), "a public IP is refused");

        // userinfo must not fool the host check: the real client connects to evil.com, not the loopback/tailnet
        assert!(!acceptable_url("http://127.0.0.1@evil.com/x"), "userinfo bypass with loopback is refused");
        assert!(!acceptable_url("https://legit.ts.net@evil.com/"), "userinfo bypass with tailnet is refused");
        assert!(!acceptable_url("http://localhost@evil.com"), "userinfo bypass with localhost is refused");
    }
}
