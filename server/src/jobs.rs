//! Work that outlives one request: drafting the day and extracting to-dos run on background threads.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use chrono::Local;
use serde_json::{Value, json};

use crate::assistant::{diary, summary, todos};
use crate::config::Paths;
use crate::llm::Model;

/// A day's draft being made, or why the last attempt failed.
enum Draft {
    Running,
    Failed(String),
}

#[derive(Default)]
pub struct Jobs {
    drafts: Mutex<HashMap<String, Draft>>,
    extracting: AtomicBool,
}

impl Jobs {
    /// The day's stored draft -> (status, body). Starts making it in the background when it is missing or stale.
    /// A failure is reported once, then a retry is allowed.
    pub fn draft(self: &Arc<Self>, paths: &Paths, model: &Arc<dyn Model>, day: &str, refresh: bool) -> (u16, Value) {
        let mut drafts = self.drafts.lock().unwrap_or_else(|e| e.into_inner());
        match drafts.get(day) {
            Some(Draft::Running) => return (202, json!({"status": "generating"})),
            Some(Draft::Failed(e)) => {
                let e = e.clone();
                drafts.remove(day);
                return (502, json!({"error": e}));
            }
            None => {}
        }
        let d = diary::read_day(paths, day);
        if d["exists"] == json!(true) && d["stale"] != json!(true) && !refresh {
            return (200, d);
        }
        drafts.insert(day.to_string(), Draft::Running);
        drop(drafts);
        let (me, paths, model, day) = (self.clone(), paths.clone(), model.clone(), day.to_string());
        std::thread::spawn(move || {
            let result = summary::generate(&paths, model.as_ref(), &day, false);
            let mut drafts = me.drafts.lock().unwrap_or_else(|e| e.into_inner());
            match result {
                Ok(_) => drafts.remove(&day),
                Err(e) => drafts.insert(day, Draft::Failed(format!("LLM request failed: {e}"))),
            };
        });
        (202, json!({"status": "generating"}))
    }

    /// For tests: pretend the last attempt for this day failed.
    pub fn set_failed(&self, day: &str, why: &str) {
        self.drafts.lock().unwrap_or_else(|e| e.into_inner()).insert(day.to_string(), Draft::Failed(why.to_string()));
    }

    /// Is a draft for this day being made or waiting to report a failure?
    pub fn pending(&self, day: &str) -> bool {
        self.drafts.lock().unwrap_or_else(|e| e.into_inner()).contains_key(day)
    }

    /// Extract new to-dos in the background, one run at a time (throttled inside).
    pub fn extract_todos(self: &Arc<Self>, paths: &Paths, model: &Arc<dyn Model>) {
        if self.extracting.swap(true, Ordering::SeqCst) {
            return;
        }
        let (me, paths, model) = (self.clone(), paths.clone(), model.clone());
        std::thread::spawn(move || {
            let today = Local::now().format("%Y-%m-%d").to_string();
            if let Err(e) = todos::extract(&paths, model.as_ref(), &today, false) {
                eprintln!("todo extract failed: {e}"); // the next request retries; nothing for the client to wait on
            }
            me.extracting.store(false, Ordering::SeqCst);
        });
    }

    pub fn todos(&self, paths: &Paths) -> Value {
        let mut v = todos::view(paths);
        v["extracting"] = json!(self.extracting.load(Ordering::SeqCst));
        v
    }
}
