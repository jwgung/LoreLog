//! The questions the assistant took to the librarian (#7), kept so real questions can join the evaluation: the
//! question, the words searched and where the hits are (file paths), never what the records say or what the assistant
//! answered. `<data>/librarian-asks.ndjson` (0600), one line per question; lines older than 90 days are dropped by the
//! sync. `LIBRARIAN_ASK_LOG=off` in ingest.env stops it.
use std::path::PathBuf;

use chrono::{Local, NaiveDate};
use serde_json::{Value, json};

use crate::config::{Paths, read_env_file};
use crate::fsutil;

/// How long questions are kept.
pub const KEEP_DAYS: i64 = 90;

pub fn file(paths: &Paths) -> PathBuf {
    paths.data.join("librarian-asks.ndjson")
}

pub fn enabled(paths: &Paths) -> bool {
    let v = read_env_file(&crate::host::ingest_env(paths)).get("LIBRARIAN_ASK_LOG").cloned().unwrap_or_default();
    !matches!(v.trim().to_lowercase().as_str(), "off" | "0" | "false" | "no")
}

/// Turn keeping questions on or off (in ingest.env, its other settings kept).
pub fn set_enabled(paths: &Paths, on: bool) -> Result<(), String> {
    let f = crate::host::ingest_env(paths);
    let mut cfg = read_env_file(&f);
    if on {
        cfg.remove("LIBRARIAN_ASK_LOG");
    } else {
        cfg.insert("LIBRARIAN_ASK_LOG".into(), "off".into());
    }
    let text: String = cfg.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
    let fail = |e: std::io::Error| format!("저장하지 못했습니다: {:?}", e.kind());
    fsutil::ensure_private_dir(&paths.conf).map_err(fail)?;
    fsutil::write_private_atomic(&f, text.as_bytes()).map_err(fail)
}

/// What became of one question.
#[derive(Clone, Debug, PartialEq)]
pub enum Found {
    /// the model chose not to look anything up
    NotSearched,
    Hits(Vec<String>),
    /// the index could not be read
    Unavailable,
}

/// Keep one question (unless turned off). A failure is only noted: it never breaks the chat.
pub fn record(paths: &Paths, day: &str, q: &str, words: Option<&[String]>, found: &Found) {
    if !enabled(paths) {
        return;
    }
    let (state, refs): (&str, &[String]) = match found {
        Found::NotSearched => ("not searched", &[]),
        Found::Hits(r) if r.is_empty() => ("nothing", &[]),
        Found::Hits(r) => ("found", r),
        Found::Unavailable => ("unavailable", &[]),
    };
    let line = json!({"ts": Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string(), "day": day, "q": q,
                      "words": words, "found": state, "refs": refs});
    let write = fsutil::ensure_private_dir(&paths.data)
        .and_then(|_| fsutil::append_private(&file(paths), &format!("{line}\n")));
    if let Err(e) = write {
        log::warn!("keeping the question failed: {:?}", e.kind());
    }
}

/// Drop the questions older than `keep_days` before `today` (and lines that are not questions). -> how many went.
pub fn prune(paths: &Paths, today: &str, keep_days: i64) -> std::io::Result<usize> {
    let f = file(paths);
    let Ok(text) = std::fs::read_to_string(&f) else { return Ok(0) };
    let Ok(today) = NaiveDate::parse_from_str(today, "%Y-%m-%d") else { return Ok(0) };
    let cut = today - chrono::TimeDelta::days(keep_days);
    let keep = |l: &str| {
        serde_json::from_str::<Value>(l)
            .ok()
            .and_then(|v| v["day"].as_str().and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok()))
            .is_some_and(|d| d >= cut)
    };
    let all: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let kept: Vec<&str> = all.iter().copied().filter(|l| keep(l)).collect();
    let dropped = all.len() - kept.len();
    if dropped > 0 {
        let body: String = kept.iter().map(|l| format!("{l}\n")).collect();
        fsutil::write_private_atomic(&f, body.as_bytes())?;
    }
    Ok(dropped)
}
