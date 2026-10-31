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
//!   3. **The server, in host mode** — when the user keeps the vault on this PC (docs/adr/0002), the app runs the
//!      server library itself (`host.rs`): receiving on loopback and the tailnet address, syncing every hour, the
//!      librarian. The screens and the collector then talk to it on loopback like to any server.
//!
//! The shell owns no judgement about the diary: it exposes the config to the UI as commands and runs the collector.
mod host;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use life_agent_core::config::{self, Mode};
use life_server::collect::sources;
use life_server::host::Models;
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager};
use tauri_plugin_autostart::ManagerExt;

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
    /// "host", "connect", or null before the user chose where the vault lives
    mode: Option<Mode>,
    base: String,
    token: String,
    /// host mode: why the app's own server is not running (the screen shows it instead of a chat that cannot work)
    host_error: Option<String>,
}

/// The server this app runs in host mode, once started, or why it could not start.
struct HostSlot(Mutex<Result<Option<host::Hosting>, String>>);

impl Default for HostSlot {
    fn default() -> Self {
        HostSlot(Mutex::new(Ok(None)))
    }
}

fn slot(s: &HostSlot) -> std::sync::MutexGuard<'_, Result<Option<host::Hosting>, String>> {
    s.0.lock().unwrap_or_else(|e| e.into_inner())
}

fn hosting<T>(s: &HostSlot, f: impl FnOnce(&host::Hosting) -> T) -> Option<T> {
    slot(s).as_ref().ok().and_then(|h| h.as_ref()).map(f)
}

/// The app-wide stop flag (set when the app exits).
struct Stop(Arc<AtomicBool>);

/// Start the server unless it runs. A failure is kept (the screen shows it) and returned.
fn start_hosting(s: &HostSlot, stop: &Arc<AtomicBool>) -> Result<(), String> {
    let mut held = slot(s);
    if matches!(*held, Ok(Some(_))) {
        return Ok(());
    }
    let started = config::load().and_then(|mut cfg| host::Hosting::start(&mut cfg, stop.clone()));
    let result = started.as_ref().map(|_| ()).map_err(Clone::clone);
    *held = started.map(Some);
    result
}

/// Where the vault lives, chosen on the first run: "host" (this PC; the app runs the server) or "connect" (a server
/// elsewhere; the screen then asks for its address and token). Switching later does not move any data.
#[tauri::command]
fn choose_mode(mode: Mode, slot: tauri::State<HostSlot>, stop: tauri::State<Stop>) -> Result<(), String> {
    if mode == Mode::Host {
        start_hosting(&slot, &stop.0)?; // the choice is kept only once the server runs (it wrote url and token)
        let mut cfg = config::load()?;
        cfg.mode = Some(Mode::Host);
        return config::save(&cfg);
    }
    // Connect: the vault stays where it is; this app stops serving it and asks for another server's address and
    // token (its own token is not offered to that server)
    let mut cfg = config::load()?;
    let was_host = cfg.mode == Some(Mode::Host);
    cfg.mode = Some(Mode::Connect);
    if was_host {
        cfg.url.clear();
        cfg.token.clear();
        config::clear_token()?;
    }
    config::save(&cfg)?;
    let mut held = self::slot(&slot);
    if let Ok(Some(h)) = &*held {
        h.stop();
    }
    *held = Ok(None);
    Ok(())
}

/// The vault's paths when this app runs the server, else an error for the settings screen.
fn host_paths(s: &HostSlot) -> Result<life_server::config::Paths, String> {
    hosting(s, |h| h.paths().clone()).ok_or_else(|| "이 PC에 금고를 둔 경우에만 설정합니다".to_string())
}

/// What is collected (the mail account without its password, the calendars).
#[tauri::command]
fn sources_view(slot: tauri::State<HostSlot>) -> Result<sources::SourcesView, String> {
    Ok(sources::view(&host_paths(&slot)?))
}

/// Log in to the mail server and list its folders (nothing saved). An empty password uses the stored one.
#[tauri::command]
async fn mail_folders(
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    slot: tauri::State<'_, HostSlot>,
) -> Result<Vec<sources::Folder>, String> {
    let paths = host_paths(&slot)?;
    blocking(move || sources::mail_folders(&paths, &host, port, &user, &password.filter(|p| !p.is_empty()))).await
}

/// Save the mail account and folders, then sync.
#[tauri::command]
fn mail_save(
    host: String,
    port: u16,
    user: String,
    password: Option<String>,
    folders: Vec<String>,
    slot: tauri::State<HostSlot>,
) -> Result<(), String> {
    let password = password.filter(|p| !p.is_empty());
    sources::save_mail(&host_paths(&slot)?, &sources::MailSetup { host, port, user, password, folders })?;
    hosting(&slot, |h| h.sync_now());
    Ok(())
}

/// The calendars on a CalDAV server, with the mail account (nothing saved).
#[tauri::command]
async fn calendar_list(server: String, slot: tauri::State<'_, HostSlot>) -> Result<Vec<sources::Calendar>, String> {
    let paths = host_paths(&slot)?;
    blocking(move || sources::calendars(&paths, &server)).await
}

/// Save the calendars to collect, then sync.
#[tauri::command]
fn calendar_save(urls: Vec<String>, slot: tauri::State<HostSlot>) -> Result<(), String> {
    sources::save_calendars(&host_paths(&slot)?, &urls)?;
    hosting(&slot, |h| h.sync_now());
    Ok(())
}

/// The address books on a CardDAV server, with the mail account (nothing saved).
#[tauri::command]
async fn book_list(server: String, slot: tauri::State<'_, HostSlot>) -> Result<Vec<sources::Book>, String> {
    let paths = host_paths(&slot)?;
    blocking(move || sources::books(&paths, &server)).await
}

/// Save the address books to collect, then sync.
#[tauri::command]
fn book_save(urls: Vec<String>, slot: tauri::State<HostSlot>) -> Result<(), String> {
    sources::save_books(&host_paths(&slot)?, &urls)?;
    hosting(&slot, |h| h.sync_now());
    Ok(())
}

/// The model addresses written in the settings.
#[tauri::command]
fn models_get(slot: tauri::State<HostSlot>) -> Result<Models, String> {
    Ok(life_server::host::saved_models(&host_paths(&slot)?))
}

/// Save the model addresses; they apply from the next question on.
#[tauri::command]
fn models_save(models: Models, slot: tauri::State<HostSlot>) -> Result<(), String> {
    hosting(&slot, |h| h.set_models(&models)).unwrap_or_else(|| Err("이 PC에 금고를 둔 경우에만 설정합니다".into()))
}

/// Are the questions the assistant takes to the librarian kept (to measure it)?
#[tauri::command]
fn asks_get(slot: tauri::State<HostSlot>) -> Result<bool, String> {
    Ok(life_server::librarian::asks::enabled(&host_paths(&slot)?))
}

#[tauri::command]
fn asks_set(on: bool, slot: tauri::State<HostSlot>) -> Result<(), String> {
    life_server::librarian::asks::set_enabled(&host_paths(&slot)?, on)
}

/// Take the vault out (#11): one zip in the user's Downloads folder (else their home). -> where it is
#[tauri::command]
async fn export_vault(slot: tauri::State<'_, HostSlot>) -> Result<String, String> {
    let paths = host_paths(&slot)?;
    blocking(move || {
        let dir = dirs::download_dir().or_else(dirs::home_dir).ok_or("내려받기 폴더를 찾지 못했습니다")?;
        let out = dir.join(format!("lorelog-export-{}.zip", chrono::Local::now().format("%Y-%m-%d-%H%M")));
        life_server::export::export(&paths, &out).map_err(|e| format!("내보내지 못했습니다: {e}"))?;
        Ok(out.to_string_lossy().into_owned())
    })
    .await
}

/// Network calls (IMAP, CalDAV) off the UI thread.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(f).await.map_err(|e| e.to_string())?
}

/// What the screens show about the server this app runs (null when it runs none).
#[tauri::command]
fn host_info(slot: tauri::State<HostSlot>) -> Option<host::Info> {
    hosting(&slot, |h| h.info())
}

/// Sync mail and calendars now (host mode).
#[tauri::command]
fn sync_now(slot: tauri::State<HostSlot>) -> Result<(), String> {
    hosting(&slot, |h| h.sync_now()).ok_or_else(|| "이 PC에 금고를 둔 경우에만 동기화합니다".to_string())
}

/// The last result of every sync job (host mode).
#[tauri::command]
fn sync_status(slot: tauri::State<HostSlot>) -> Option<serde_json::Value> {
    hosting(&slot, |h| h.status())
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
fn server_config(slot: tauri::State<HostSlot>) -> Result<ServerConfig, String> {
    let cfg = config::load()?;
    let host_error = if cfg.mode == Some(Mode::Host) {
        match &*self::slot(&slot) {
            Ok(Some(_)) => None,
            Ok(None) => Some("서버가 아직 시작되지 않았습니다".to_string()),
            Err(e) => Some(e.clone()),
        }
    } else {
        None
    };
    Ok(ServerConfig { mode: cfg.mode, base: cfg.url.trim_end_matches('/').to_string(), token: cfg.token, host_error })
}

/// Save the server address and (if given) the token. An empty token leaves the stored one untouched. The address
/// must be a tailnet/loopback host (see `config::is_acceptable_url`) so the token is never sent to a public host.
#[tauri::command]
fn save_settings(url: String, token: Option<String>) -> Result<(), String> {
    if config::load()?.mode == Some(Mode::Host) {
        return Err("이 PC에 금고를 둔 동안에는 이 앱의 서버를 씁니다. 다른 서버로 바꾸려면 금고 위치를 바꾸세요".into());
    }
    let url = url.trim().trim_end_matches('/').to_string();
    if !config::is_acceptable_url(&url) {
        return Err("주소는 tailnet(*.ts.net) 이어야 합니다 (http는 로컬/테일넷만)".into());
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

/// The argument the OS starts the app with at login: it then stays in the tray instead of opening its window.
const AUTOSTART_ARG: &str = "--autostart";

/// Does the app start when the user logs in? The OS keeps the answer (Windows: the Run key; macOS: a launch agent;
/// Linux: an autostart entry), so it is read from there, not from our config.
#[tauri::command]
fn get_autostart(app: AppHandle) -> Result<bool, String> {
    app.autolaunch().is_enabled().map_err(|e| format!("시작 프로그램 설정을 읽지 못했습니다: {e}"))
}

/// Turn starting at login on or off. Off by default: the user chooses.
#[tauri::command]
fn set_autostart(app: AppHandle, on: bool) -> Result<bool, String> {
    let m = app.autolaunch();
    let r = if on { m.enable() } else { m.disable() };
    r.map_err(|e| format!("시작 프로그램 설정을 바꾸지 못했습니다: {e}"))?;
    m.is_enabled().map_err(|e| format!("시작 프로그램 설정을 읽지 못했습니다: {e}"))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let started_at_login = std::env::args().any(|a| a == AUTOSTART_ARG);
    let stop = Arc::new(AtomicBool::new(false));
    let stop_for_exit = stop.clone();
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARG]),
        ))
        .invoke_handler(tauri::generate_handler![
            get_autostart,
            set_autostart,
            get_settings,
            server_config,
            save_settings,
            add_watch,
            remove_watch,
            choose_mode,
            host_info,
            sync_now,
            sync_status,
            sources_view,
            mail_folders,
            mail_save,
            calendar_list,
            calendar_save,
            book_list,
            book_save,
            models_get,
            models_save,
            asks_get,
            export_vault,
            asks_set
        ])
        .manage(HostSlot::default())
        .manage(Stop(stop.clone()))
        .setup(move |app| {
            build_tray(app.handle())?;
            if config::load().ok().and_then(|c| c.mode) == Some(Mode::Host)
                && let Err(e) = start_hosting(&app.state::<HostSlot>(), &stop)
            {
                // kept in the slot too: the screen shows it
                life_agent_core::agent::log(&config::home(), &format!("server not running: {e}"));
            }
            if started_at_login && let Some(w) = app.get_webview_window("main") {
                let _ = w.hide(); // started at login: collect in the tray, open when the user asks
            }
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
        .run(move |app, event| {
            if let tauri::RunEvent::ExitRequested { .. } = event {
                stop_for_exit.store(true, Ordering::SeqCst);
                hosting(&app.state::<HostSlot>(), |h| h.stop());
            }
        });
}
