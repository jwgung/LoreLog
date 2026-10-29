//! Watched-folder intake: files the user dropped into a folder they chose are kept by the server as deliverables.
//!
//! The rule that matters: **a file is never lost and never sent twice.** The original stays in place — we store a
//! copy on the server (the vault), we do not move or delete the user's file. A file is sent only after it has held
//! still (`SETTLE`), is hashed once, and is marked done in the journal only after the server confirmed it kept the
//! bytes. If the upload fails the next scan picks the same file up again. The journal (`intake-state.json`) is written
//! atomically after each change so a restart does not re-send a kept file.
//!
//! Only top-level files are taken; subfolders are not descended (that is the user's own structure). Work-in-progress,
//! hidden and OS files are skipped. The server address and token live in the sender, not here — the uploader is
//! injected so this is tested without a server.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// A file must hold the same size and mtime for this long before it is read (it may still be downloading or saving).
pub const SETTLE_S: f64 = 10.0;

/// A file larger than this is not read or uploaded (it would be refused by the server anyway). It matches the
/// server's default request limit; the server has the final say and a rejection is remembered either way.
pub const MAX_UPLOAD_BYTES: u64 = 20 * 1024 * 1024;

/// What a put returned.
pub struct Uploaded {
    pub duplicate: bool,
}

/// Why an upload did not succeed. A transient problem (offline, 5xx) stops the pass and is retried; a rejection
/// (the server refused this file for good — too large, malformed) is remembered so one bad file does not block the
/// rest forever.
pub enum UploadError {
    Transient(String),
    Rejected(String),
}

/// The server side: ask whether a blob is already kept, send one file, or record a sighting of bytes already kept.
/// Injected so the engine is testable without a server.
pub trait Uploader {
    fn has(&self, sha: &str) -> Result<bool, String>;
    fn put(&self, sha: &str, name: &str, mime: &str, data: &[u8], when_ms: i64) -> Result<Uploaded, UploadError>;
    /// Record that this (already-kept) file was seen, without re-sending the bytes.
    fn sight(&self, sha: &str, name: &str, mime: &str, when_ms: i64) -> Result<(), UploadError>;
}

/// What the journal remembers about one file (keyed by its absolute path).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct Entry {
    size: u64,
    /// mtime as epoch nanoseconds (an integer, so it round-trips through JSON exactly); with size it tells whether
    /// the file changed.
    mtime_ns: i64,
    /// Wall time (epoch seconds) when size+mtime were last seen unchanged. Used for the settle check.
    stable_since: f64,
    /// Set once the server has kept this exact content and a sighting was recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    done_sha: Option<String>,
    /// Set when the server refused this file for good (too large, malformed). It is not retried, and it does not
    /// block the files after it. Cleared if the file changes (a new deliverable).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    rejected: bool,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Journal {
    /// path string -> entry
    files: BTreeMap<String, Entry>,
}

pub struct IntakeEngine {
    path: PathBuf,
    journal: Journal,
    settle_s: f64,
    max_bytes: u64,
}

fn key(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

/// Names we never take: work-in-progress downloads, office locks, hidden and OS housekeeping files.
fn skip_name(name: &str) -> bool {
    let lower = name.to_lowercase();
    if name.starts_with('.') || name.starts_with("~$") {
        return true;
    }
    if matches!(lower.as_str(), "desktop.ini" | "thumbs.db" | "ehthumbs.db") {
        return true;
    }
    const WIP: &[&str] = &[
        ".crdownload", ".part", ".partial", ".download", ".opdownload", ".tmp", ".temp", ".!ut", ".aria2",
    ];
    WIP.iter().any(|s| lower.ends_with(s))
}

fn mtime_ns(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_nanos() as i64)
}

pub fn system_wall() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

impl IntakeEngine {
    /// Open (or start) the journal at `path`, using the default settle time.
    pub fn open(path: &Path) -> Result<Self, String> {
        Self::open_with_settle(path, SETTLE_S)
    }

    /// Open with a chosen settle time (from the config).
    pub fn open_with_settle(path: &Path, settle_s: f64) -> Result<Self, String> {
        let journal = match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Journal::default(),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        Ok(IntakeEngine {
            path: path.to_path_buf(),
            journal,
            settle_s: if settle_s > 0.0 { settle_s } else { SETTLE_S },
            max_bytes: MAX_UPLOAD_BYTES,
        })
    }

    /// The largest file this engine will read and upload. A file over this is skipped (and remembered), not retried.
    pub fn with_max_bytes(mut self, max_bytes: u64) -> Self {
        if max_bytes > 0 {
            self.max_bytes = max_bytes;
        }
        self
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(&self.journal).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("tmp");
        std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
        std::fs::rename(&tmp, &self.path).map_err(|e| e.to_string())
    }

    /// Has this path been kept by the server?
    pub fn is_done(&self, p: &Path) -> bool {
        self.journal.files.get(&key(p)).is_some_and(|e| e.done_sha.is_some())
    }

    /// Look at the top-level files of each watched folder and update the journal's size/mtime/stable-since.
    /// A file whose size or mtime changed resets its settle clock. Entries for files that vanished are forgotten
    /// (unless already kept — those stay so a re-drop of the same name is still noticed as new only if its bytes differ).
    pub fn scan(&mut self, dirs: &[PathBuf], now: f64) {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for dir in dirs {
            let entries = match std::fs::read_dir(dir) {
                Ok(e) => e,
                Err(_) => continue, // folder not there yet, or not readable: nothing to take this pass
            };
            for de in entries.flatten() {
                let path = de.path();
                let meta = match de.metadata() {
                    Ok(m) => m,
                    Err(_) => continue,
                };
                if !meta.is_file() {
                    continue; // subfolders are not descended
                }
                let name = match path.file_name().and_then(|n| n.to_str()) {
                    Some(n) => n.to_string(),
                    None => continue,
                };
                if skip_name(&name) {
                    continue;
                }
                let k = key(&path);
                seen.insert(k.clone());
                let (size, mtime) = (meta.len(), mtime_ns(&meta));
                let entry = self.journal.files.entry(k).or_insert(Entry {
                    size,
                    mtime_ns: mtime,
                    stable_since: now,
                    done_sha: None,
                    rejected: false,
                });
                if entry.size != size || entry.mtime_ns != mtime {
                    // the file changed: it is a new deliverable; reset the settle clock, the kept mark and any rejection
                    entry.size = size;
                    entry.mtime_ns = mtime;
                    entry.stable_since = now;
                    entry.done_sha = None;
                    entry.rejected = false;
                }
            }
        }
        // forget entries for files that are no longer in any watched folder
        self.journal.files.retain(|k, _| seen.contains(k));
        let _ = self.save();
    }

    /// Upload the files that have settled and are not yet kept. Each kept file is marked only after the server
    /// confirmed it, and the journal is written after every change. A failure stops this pass and leaves the rest
    /// for the next one.
    pub fn step(&mut self, up: &dyn Uploader, now: f64) {
        let ready: Vec<String> = self
            .journal
            .files
            .iter()
            .filter(|(_, e)| e.done_sha.is_none() && !e.rejected && now - e.stable_since >= self.settle_s)
            .map(|(k, _)| k.clone())
            .collect();
        for k in ready {
            let path = PathBuf::from(&k);
            let meta = match std::fs::metadata(&path) {
                Ok(m) => m,
                Err(_) => continue, // vanished between scan and now: next scan reconciles
            };
            // too big to send (the server would refuse it): remember that and move on, do not read it into memory
            if meta.len() > self.max_bytes {
                self.mark_rejected(&k);
                continue;
            }
            // an empty file has nothing to keep; skip it without a wasted upload+413, and do not churn on it
            if meta.len() == 0 {
                self.mark_rejected(&k);
                continue;
            }
            let data = match std::fs::read(&path) {
                Ok(d) => d,
                Err(_) => continue, // unreadable right now (locked by another program): try next pass
            };
            // if it changed since the scan, do not send a torn read; let the next scan reset the clock
            if let Some(e) = self.journal.files.get(&k)
                && (e.size != meta.len() || e.mtime_ns != mtime_ns(&meta))
            {
                continue;
            }
            let sha = {
                let mut h = Sha256::new();
                h.update(&data);
                format!("{:x}", h.finalize())
            };
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("(unnamed)").to_string();
            let mime = "application/octet-stream";
            // the file's own modified time, in epoch ms; fall back to now if the OS gives nothing
            let when_ms = if mtime_ns(&meta) > 0 {
                mtime_ns(&meta) / 1_000_000
            } else {
                (now * 1000.0) as i64
            };
            // skip the bytes if the server already keeps them, but still record the sighting; if the sighting fails
            // because the blob has since gone, fall back to a full upload
            let already = up.has(&sha).unwrap_or(false);
            let result = if already {
                match up.sight(&sha, &name, mime, when_ms) {
                    Ok(()) => Ok(()),
                    Err(UploadError::Rejected(_)) => up.put(&sha, &name, mime, &data, when_ms).map(|_| ()),
                    Err(e) => Err(e),
                }
            } else {
                up.put(&sha, &name, mime, &data, when_ms).map(|_| ())
            };
            match result {
                Ok(()) => {
                    if let Some(e) = self.journal.files.get_mut(&k) {
                        e.done_sha = Some(sha);
                    }
                    let _ = self.save();
                }
                Err(UploadError::Rejected(_)) => self.mark_rejected(&k), // refused for good: skip, do not block the rest
                Err(UploadError::Transient(_)) => break, // offline or a 5xx: stop, keep the rest for next time
            }
        }
    }

    fn mark_rejected(&mut self, k: &str) {
        if let Some(e) = self.journal.files.get_mut(k) {
            e.rejected = true;
        }
        let _ = self.save();
    }
}
