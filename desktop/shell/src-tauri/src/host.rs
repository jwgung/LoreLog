//! Host mode: the vault lives on this PC and the app runs the server itself (docs/adr/0002). The same library as the
//! home server: it receives on loopback (this app's screen and PC collector) and on the tailnet address (phones and
//! other PCs), syncs mail and calendars every hour, and keeps the librarian's index. Settings and data sit where the
//! server keeps them on this OS, so the command line tools on the same PC see the same vault.
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use life_agent_core::config::{self, Config};
use life_server::collect::{setup, sync};
use life_server::config::{OsDirs, Paths};
use life_server::host::{self, IngestSettings, Models, Running};
use life_server::http::App;
use serde::Serialize;

const SYNC_EVERY: Duration = Duration::from_secs(3600);
const TAILNET_RETRY: Duration = Duration::from_secs(30);
const LOG_MAX: u64 = 5 * 1024 * 1024;

/// Where the vault is: the server's places on this OS; notes in the user's chosen folder, else the app's own.
pub fn paths(cfg: &Config) -> Paths {
    let os = OsDirs::here();
    let (conf, data) = (os.conf_dir(), os.data_dir());
    let vault = cfg.vault_dir.as_ref().filter(|d| !d.trim().is_empty()).map(PathBuf::from);
    Paths { vault: vault.unwrap_or_else(|| data.join("notes")), allow_file: conf.join("diary.json"), data, conf }
}

/// The library's notes go to `<data>/desktop.log` (a desktop program has no console). Never the token.
fn log_to_file(data: &Path) {
    let file = data.join("desktop.log");
    life_server::logging::init(move |m| {
        use std::io::Write;
        if std::fs::metadata(&file).is_ok_and(|md| md.len() > LOG_MAX) {
            let _ = std::fs::write(&file, "");
        }
        if let Ok(mut f) = life_server::fsutil::open_append_private(&file) {
            let _ = writeln!(f, "{} {m}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"));
        }
    });
}

/// This PC's tailnet addresses, if Tailscale is up.
fn tailnet_addrs() -> Vec<IpAddr> {
    let mut out: Vec<IpAddr> = if_addrs::get_if_addrs()
        .map(|ifs| ifs.into_iter().map(|i| i.ip()).filter(|ip| host::is_tailnet_ip(*ip)).collect())
        .unwrap_or_default();
    out.sort();
    out.dedup();
    out
}

/// This PC's name on the tailnet (`pc.tailnet.ts.net`), asked from the tailscale command if it is there.
fn tailnet_name() -> Option<String> {
    let mut cmd = std::process::Command::new("tailscale");
    cmd.args(["status", "--json"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console flashing up
    }
    let out = cmd.output().ok().filter(|o| o.status.success())?;
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).ok()?;
    let name = v["Self"]["DNSName"].as_str()?.trim_end_matches('.').to_string();
    (!name.is_empty()).then_some(name)
}

/// What the screens show about the server this app runs.
#[derive(Serialize)]
pub struct Info {
    /// where it listens, e.g. "127.0.0.1:8765", "100.101.1.2:8765"
    pub listening: Vec<String>,
    /// addresses it could not listen on, and why
    pub failed: Vec<String>,
    /// the address other devices put in, when the PC is on a tailnet
    pub address_for_devices: Option<String>,
    pub notes_dir: String,
    pub syncing: bool,
}

/// The server running in this app.
pub struct Hosting {
    rt: tokio::runtime::Runtime,
    app: Arc<App>,
    paths: Paths,
    port: u16,
    running: Arc<Mutex<Vec<Running>>>,
    sync_now: mpsc::Sender<()>,
    syncing: Arc<AtomicBool>,
}

impl Hosting {
    /// Start it: make the token on first use, listen on loopback (an error if even that fails), point this app's
    /// screen and PC collector at it, and keep trying the tailnet address and syncing every hour until `stop`.
    pub fn start(cfg: &mut Config, stop: Arc<AtomicBool>) -> Result<Hosting, String> {
        let paths = paths(cfg);
        life_server::fsutil::ensure_private_dir(&paths.data).map_err(|e| format!("데이터 폴더를 만들지 못했습니다: {e}"))?;
        log_to_file(&paths.data);
        let token = host::ensure_token(&paths).map_err(|e| format!("토큰을 만들지 못했습니다: {e}"))?;
        let settings = IngestSettings::load(&paths)?;
        let app = Arc::new(Models::load(&paths).app(paths.clone(), &settings));
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("life-host")
            .build()
            .map_err(|e| format!("서버를 띄우지 못했습니다: {e}"))?;
        let local = host::start(rt.handle(), app.clone(), settings.port, &[IpAddr::V4(Ipv4Addr::LOCALHOST)]);
        let Some(addr) = local.listening().first().copied() else {
            let why = local.failed().first().map(|(_, w)| w.clone()).unwrap_or_default();
            return Err(format!("포트 {}를 열지 못했습니다({why}). 다른 프로그램이 쓰고 있을 수 있습니다", settings.port));
        };
        cfg.url = format!("http://{addr}");
        cfg.token = token;
        config::save(cfg)?;
        let (sync_now, wake) = mpsc::channel();
        let hosting = Hosting {
            rt,
            app,
            paths,
            port: addr.port(),
            running: Arc::new(Mutex::new(vec![local])),
            sync_now,
            syncing: Arc::new(AtomicBool::new(false)),
        };
        hosting.keep_tailnet(stop.clone());
        hosting.keep_syncing(stop, wake);
        Ok(hosting)
    }

    /// Listen on the tailnet address too as soon as there is one (Tailscale may come up after the app).
    fn keep_tailnet(&self, stop: Arc<AtomicBool>) {
        let (rt, app, port, running) = (self.rt.handle().clone(), self.app.clone(), self.port, self.running.clone());
        std::thread::spawn(move || {
            let mut tried: Vec<IpAddr> = Vec::new();
            while !stop.load(Ordering::SeqCst) {
                let fresh: Vec<IpAddr> = tailnet_addrs().into_iter().filter(|a| !tried.contains(a)).collect();
                if !fresh.is_empty() {
                    let r = host::start(&rt, app.clone(), port, &fresh);
                    for (ip, why) in r.failed() {
                        log::warn!(target: "life_server::desktop", "cannot listen on {ip}:{port}: {why}");
                    }
                    tried.extend(r.listening().iter().map(|a| a.ip()));
                    running.lock().unwrap_or_else(|e| e.into_inner()).push(r);
                }
                sleep_unless(&stop, TAILNET_RETRY);
            }
        });
    }

    /// Sync now, then every hour, and whenever asked ([`Hosting::sync_now`]), until `stop`.
    fn keep_syncing(&self, stop: Arc<AtomicBool>, wake: mpsc::Receiver<()>) {
        let (paths, syncing) = (self.paths.clone(), self.syncing.clone());
        std::thread::spawn(move || {
            loop {
                syncing.store(true, Ordering::SeqCst);
                let r = sync::sync(&paths, false, &mut |job, outcome, lines| {
                    if outcome != sync::Outcome::Ok {
                        log::warn!(target: "life_server::desktop", "{}: {}", job.name(), sync::detail(outcome, lines));
                    }
                });
                syncing.store(false, Ordering::SeqCst);
                let _ = r; // nothing configured yet is fine: the user adds mail or a calendar in the settings
                let until = Instant::now() + SYNC_EVERY;
                loop {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    if wake.recv_timeout(Duration::from_secs(1)).is_ok() || Instant::now() >= until {
                        break;
                    }
                }
            }
        });
    }

    /// Ask for a sync now (one waiting request is enough; a running sync is not doubled).
    pub fn sync_now(&self) {
        if !self.syncing.load(Ordering::SeqCst) {
            let _ = self.sync_now.send(());
        }
    }

    /// The last result of every sync job, as `life status` shows it.
    pub fn status(&self) -> serde_json::Value {
        let configured = setup::configured(&life_server::collect::settings::read_sources(&self.paths));
        let st = setup::load_status(&sync::status_file(&self.paths));
        let jobs: Vec<serde_json::Value> = configured
            .iter()
            .map(|n| {
                let j = st.get(*n).cloned().unwrap_or(serde_json::Value::Null);
                serde_json::json!({"name": n, "result": j})
            })
            .collect();
        serde_json::json!({"syncing": self.syncing.load(Ordering::SeqCst), "jobs": jobs})
    }

    pub fn info(&self) -> Info {
        let running = self.running.lock().unwrap_or_else(|e| e.into_inner());
        let listening: Vec<String> = running.iter().flat_map(|r| r.listening().iter().map(|a| a.to_string())).collect();
        let failed = running.iter().flat_map(|r| r.failed().iter().map(|(ip, w)| format!("{ip}: {w}"))).collect();
        let on_tailnet = running.iter().flat_map(|r| r.listening()).find(|a| host::is_tailnet_ip(a.ip())).copied();
        let address_for_devices = on_tailnet.map(|a| {
            let host = tailnet_name().unwrap_or_else(|| a.ip().to_string());
            format!("http://{host}:{}", a.port())
        });
        Info {
            listening,
            failed,
            address_for_devices,
            notes_dir: self.paths.vault.to_string_lossy().to_string(),
            syncing: self.syncing.load(Ordering::SeqCst),
        }
    }

    /// Stop listening (requests in flight finish). The sync and tailnet threads end with the app's stop flag.
    pub fn stop(&self) {
        for r in self.running.lock().unwrap_or_else(|e| e.into_inner()).iter() {
            r.stop();
        }
    }
}

fn sleep_unless(stop: &AtomicBool, d: Duration) {
    let until = Instant::now() + d;
    while Instant::now() < until && !stop.load(Ordering::SeqCst) {
        std::thread::sleep(Duration::from_millis(500));
    }
}

// Linux only: the OS places are moved with XDG_* for the test (process-wide, so this is the crate's only such test).
#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn host_mode_serves_on_loopback_points_the_app_at_it_and_stops() {
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", dir.path().join("config"));
            std::env::set_var("XDG_DATA_HOME", dir.path().join("data"));
            std::env::set_var("LIFE_AGENT_HOME", dir.path().join("agent"));
        }
        let mut cfg = Config { mode: Some(config::Mode::Host), ..Config::default() };
        let p = paths(&cfg);
        assert_eq!(p.vault, dir.path().join("data/life/notes"), "notes in the app's own folder by default");
        std::fs::create_dir_all(&p.conf).unwrap();
        std::fs::write(host::ingest_env(&p), "INGEST_PORT=0\n").unwrap(); // any free port

        let stop = Arc::new(AtomicBool::new(false));
        let h = Hosting::start(&mut cfg, stop.clone()).unwrap();
        assert!(cfg.url.starts_with("http://127.0.0.1:") && cfg.token.len() >= 32, "{}", cfg.url);
        let saved = config::load().unwrap();
        assert_eq!((saved.url.as_str(), saved.token.as_str()), (cfg.url.as_str(), cfg.token.as_str()));

        let ok = ureq::get(&format!("{}/todos", cfg.url)).header("Authorization", format!("Bearer {}", cfg.token));
        assert_eq!(ok.call().unwrap().status(), 200);
        std::thread::sleep(Duration::from_millis(300)); // the tailnet listener starts on its own thread
        let info = h.info();
        assert_eq!(info.listening[0], cfg.url.trim_start_matches("http://"));
        let on_tailnet = !tailnet_addrs().is_empty();
        assert_eq!(info.listening.len() > 1, on_tailnet, "the tailnet address too, when this PC has one");
        assert_eq!(info.address_for_devices.is_some(), on_tailnet);
        assert_eq!(h.status()["jobs"], serde_json::json!([]), "nothing to sync until mail or a calendar is added");

        stop.store(true, Ordering::SeqCst);
        h.stop();
        std::thread::sleep(Duration::from_millis(300));
        assert!(ureq::get(&format!("{}/healthz", cfg.url)).call().is_err());
        unsafe {
            std::env::remove_var("XDG_CONFIG_HOME");
            std::env::remove_var("XDG_DATA_HOME");
            std::env::remove_var("LIFE_AGENT_HOME");
        }
    }
}
