//! The assistant speaks first on this PC too (#60): while this app runs the server, every half minute it asks the
//! library for what starts within ten minutes (the vault's CalDAV events) and says each line as an OS notification.
//! The rules (certain only, once per event, not the same (event, matter) again, no 22:00–07:00, stopped
//! organizations) live in `life_server::assistant::ahead`, which also keeps what the phone already said. A desktop
//! notification has no buttons, and its swipes are not seen: the phone's are the ones that stop an organization.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use life_server::assistant::ahead::{self, Ask};
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

const EVERY: Duration = Duration::from_secs(30);
const LOG_TARGET: &str = "life_server::desktop";

/// Until the app exits: say what is due (host mode only; in connect mode the phone and that server's host say it).
pub fn keep_saying(app: AppHandle, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        while !stop.load(Ordering::SeqCst) {
            if let Some(paths) = super::hosting(&app.state::<super::HostSlot>(), |h| h.paths().clone()) {
                let now = chrono::Local::now().naive_local();
                match ahead::take(&paths, &Ask::default(), now, ahead::LEAD) {
                    Ok(lines) => {
                        for l in lines {
                            if let Err(e) = app.notification().builder().title(&l.title).body(&l.text).show() {
                                log::warn!(target: LOG_TARGET, "notification failed: {e}");
                            }
                        }
                    }
                    // an index not built yet, or of another version: nothing is certain until the next sync
                    Err(e) => log::debug!(target: LOG_TARGET, "ahead: {}", life_server::errors::kind(&e)),
                }
            }
            let until = Instant::now() + EVERY;
            while Instant::now() < until && !stop.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_millis(500));
            }
        }
    });
}
