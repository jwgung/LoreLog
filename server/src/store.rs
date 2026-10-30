//! Raw store: NDJSON lines appended to `<data>/raw/<source>/<YYYY-MM-DD>.ndjson`, never edited or deleted.
//!
//! - The day comes from the record's time (`ts`, `timestamp`, `time` or `t`: epoch milliseconds or seconds, or an ISO
//!   time), else from when it arrived (server local time).
//! - A record that arrives again (a resend) is stored once. Records are compared as parsed JSON without the fields the
//!   store adds, so a resend is recognised however the sender spaced its JSON, also after a restart.
//! - Records are kept as they are; only `_src`, `_recv` and `_class` are added.
//! - Blobs (file bytes) live under their own content hash in `raw/files/<sha[:2]>/<sha>`, shared by every channel.
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, TimeZone};
use serde_json::{Map, Value};
use sha1::{Digest, Sha1};
use sha2::Sha256;

use crate::fsutil::{append_private, ensure_private_dir, set_mode, write_private_atomic};

/// Fields the store adds to every record. They are left out when two records are compared.
const ADDED: [&str; 3] = ["_src", "_recv", "_class"];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Counts {
    pub accepted: usize,
    pub duplicates: usize,
    pub rejected: usize,
}

/// A record ready to append: its comparison key and its line.
type Pending = ([u8; 20], String);

/// The raw store of one data directory. One per process: it remembers which records each day file already holds.
pub struct RawStore {
    data: PathBuf,
    seen: Mutex<HashMap<PathBuf, HashSet<[u8; 20]>>>,
}

impl RawStore {
    pub fn new(data: &Path) -> RawStore {
        RawStore { data: data.to_path_buf(), seen: Mutex::new(HashMap::new()) }
    }

    pub fn data(&self) -> &Path {
        &self.data
    }

    /// Append an NDJSON body to the raw files of `source`. Lines that are not JSON objects are counted as rejected.
    pub fn store(&self, source: &str, body: &str, cls: &str) -> io::Result<Counts> {
        let now = Local::now();
        let recv = now.format("%Y-%m-%dT%H:%M:%S%:z").to_string();
        let mut counts = Counts::default();
        let mut by_day: Vec<(NaiveDate, Vec<Pending>)> = Vec::new();
        for raw in body.lines() {
            let raw = raw.trim();
            if raw.is_empty() {
                continue;
            }
            let Ok(Value::Object(mut rec)) = serde_json::from_str::<Value>(raw) else {
                counts.rejected += 1;
                continue;
            };
            let digest = digest_of(&rec);
            let day = record_day(&rec, now.naive_local()).date();
            rec.insert("_src".into(), Value::String(source.into()));
            rec.insert("_recv".into(), Value::String(recv.clone()));
            rec.insert("_class".into(), Value::String(cls.into()));
            let line = serde_json::to_string(&Value::Object(rec)).expect("a parsed record serializes");
            match by_day.iter_mut().find(|(d, _)| *d == day) {
                Some((_, items)) => items.push((digest, line)),
                None => by_day.push((day, vec![(digest, line)])),
            }
        }
        let mut seen_all = self.seen.lock().unwrap_or_else(|e| e.into_inner());
        for (day, items) in by_day {
            let dir = self.data.join("raw").join(source);
            ensure_private_dir(&self.data)?;
            ensure_private_dir(&self.data.join("raw"))?;
            ensure_private_dir(&dir)?;
            let file = dir.join(format!("{}.ndjson", day.format("%Y-%m-%d")));
            let seen = seen_all.entry(file.clone()).or_insert_with(|| digests_in(&file));
            let mut fresh = Vec::new();
            for (digest, line) in items {
                if seen.insert(digest) {
                    fresh.push(line);
                    counts.accepted += 1;
                } else {
                    counts.duplicates += 1;
                }
            }
            if !fresh.is_empty() {
                append_private(&file, &(fresh.join("\n") + "\n"))?;
            }
        }
        Ok(counts)
    }
}

/// The comparison key of a record: SHA-1 of its compact JSON without the fields the store adds.
fn digest_of(rec: &Map<String, Value>) -> [u8; 20] {
    let mut own = rec.clone();
    for k in ADDED {
        own.remove(k);
    }
    let text = serde_json::to_string(&Value::Object(own)).expect("a parsed record serializes");
    Sha1::digest(text.as_bytes()).into()
}

/// The keys of the records a day file already holds. An unreadable or missing file holds none.
fn digests_in(file: &Path) -> HashSet<[u8; 20]> {
    let Ok(bytes) = fs::read(file) else {
        return HashSet::new();
    };
    String::from_utf8_lossy(&bytes)
        .lines()
        .filter_map(|l| match serde_json::from_str::<Value>(l) {
            Ok(Value::Object(o)) => Some(digest_of(&o)),
            _ => None,
        })
        .collect()
}

/// When a record happened, in server local time: the first of `ts`, `timestamp`, `time`, `t` that is a positive number
/// (milliseconds above 1e11, else seconds) or an ISO time. Without one (or one out of range), `now`.
pub fn record_day(rec: &Map<String, Value>, now: NaiveDateTime) -> NaiveDateTime {
    for k in ["ts", "timestamp", "time", "t"] {
        match rec.get(k) {
            Some(Value::Number(n)) => {
                let Some(v) = n.as_f64() else { continue };
                if v > 0.0 {
                    let secs = if v > 1e11 { v / 1000.0 } else { v };
                    return local_from_secs(secs).unwrap_or(now);
                }
            }
            Some(Value::String(s)) => {
                if let Some(t) = parse_iso_local(s) {
                    return t;
                }
            }
            _ => {}
        }
    }
    now
}

fn local_from_secs(secs: f64) -> Option<NaiveDateTime> {
    if !secs.is_finite() || secs > 1e13 {
        return None;
    }
    let whole = secs.floor();
    let nanos = ((secs - whole) * 1e9) as u32;
    Local.timestamp_opt(whole as i64, nanos).single().map(|t| t.naive_local())
}

/// An ISO time as local naive time: with an offset (or `Z`) it is converted, without one it is taken as local already.
pub fn parse_iso_local(s: &str) -> Option<NaiveDateTime> {
    let s = s.trim();
    let zoned = s.strip_suffix('Z').map(|r| format!("{r}+00:00")).unwrap_or_else(|| s.to_string());
    for f in ["%Y-%m-%dT%H:%M:%S%.f%:z", "%Y-%m-%d %H:%M:%S%.f%:z", "%Y-%m-%dT%H:%M%:z", "%Y-%m-%dT%H:%M:%S%.f%z"] {
        if let Ok(t) = DateTime::parse_from_str(&zoned, f) {
            return Some(t.with_timezone(&Local).naive_local());
        }
    }
    for f in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%dT%H:%M", "%Y-%m-%d %H:%M"] {
        if let Ok(t) = NaiveDateTime::parse_from_str(s, f) {
            return Some(t);
        }
    }
    NaiveDate::parse_from_str(s, "%Y-%m-%d").ok().and_then(|d| d.and_hms_opt(0, 0, 0))
}

// ---------- blobs ----------

/// Where a deliverable's bytes live: named by their own content hash, shared by every channel that sees the file.
pub fn blob_path(data: &Path, sha: &str) -> PathBuf {
    data.join("raw").join("files").join(&sha[..2.min(sha.len())]).join(sha)
}

pub fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// Write the bytes once, under their hash. -> (sha256, newly written)
pub fn put_blob(data_dir: &Path, data: &[u8]) -> io::Result<(String, bool)> {
    let sha = sha256_hex(data);
    let path = blob_path(data_dir, &sha);
    if fs::metadata(&path).map(|m| m.len() == data.len() as u64).unwrap_or(false) {
        return Ok((sha, false));
    }
    ensure_private_dir(data_dir)?;
    ensure_private_dir(&data_dir.join("raw"))?;
    ensure_private_dir(&data_dir.join("raw").join("files"))?;
    ensure_private_dir(path.parent().expect("a blob path has a parent"))?;
    write_private_atomic(&path, data)?;
    set_mode(&path, 0o600);
    Ok((sha, true))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ms(y: i32, m: u32, d: u32, h: u32, mi: u32) -> i64 {
        Local.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap().timestamp_millis()
    }

    fn lines(p: &Path) -> Vec<Value> {
        fs::read_to_string(p).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect()
    }

    #[test]
    fn records_go_to_the_day_they_happened_once_and_bad_lines_are_counted() {
        let dir = tempfile::tempdir().unwrap();
        let store = RawStore::new(dir.path());
        let ts = ms(2026, 10, 26, 13, 5);
        let body = format!(
            "{}\n{}\n이건 JSON 이 아님\n[1,2]\n\n",
            json!({"type": "voice", "ts": ts, "text": "회의 시작"}),
            json!({"type": "app", "ts": ts + 1000, "pkg": "com.x"})
        );
        assert_eq!(
            store.store("lorelog", &body, "personal").unwrap(),
            Counts { accepted: 2, duplicates: 0, rejected: 2 }
        );
        assert_eq!(
            store.store("lorelog", &body, "personal").unwrap(),
            Counts { accepted: 0, duplicates: 2, rejected: 2 }
        );
        let f = dir.path().join("raw/lorelog/2026-10-26.ndjson");
        let recs = lines(&f);
        assert_eq!(recs.len(), 2);
        assert_eq!(recs[0]["text"], "회의 시작");
        assert_eq!(recs[0]["_src"], "lorelog");
        assert_eq!(recs[0]["_class"], "personal");
        let keys: Vec<&String> = recs[0].as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["type", "ts", "text", "_src", "_recv", "_class"],
            "the record as sent, then what the store adds"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(&f).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(fs::metadata(dir.path().join("raw")).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }

    #[test]
    fn a_resend_is_recognised_after_a_restart_however_its_json_is_spaced() {
        let dir = tempfile::tempdir().unwrap();
        let ts = ms(2026, 3, 1, 9, 0);
        RawStore::new(dir.path())
            .store("pc", &format!("{{\"type\":\"window\",\"ts\":{ts},\"app\":\"한글.exe\"}}\n"), "work")
            .unwrap();
        let again = RawStore::new(dir.path()); // a new process: knows only the file
        let spaced = format!("{{ \"type\": \"window\", \"ts\": {ts}, \"app\": \"\\ud55c\\uae00.exe\" }}\n");
        assert_eq!(again.store("pc", &spaced, "work").unwrap(), Counts { accepted: 0, duplicates: 1, rejected: 0 });
    }

    #[test]
    fn the_day_comes_from_ms_seconds_or_iso_else_now() {
        let now = NaiveDate::from_ymd_opt(2030, 1, 1).unwrap().and_hms_opt(12, 0, 0).unwrap();
        let day = |v: Value| record_day(v.as_object().unwrap(), now).date().to_string();
        let ts = ms(2026, 10, 26, 23, 59);
        assert_eq!(day(json!({"ts": ts})), "2026-10-26");
        assert_eq!(day(json!({"timestamp": ts / 1000})), "2026-10-26");
        assert_eq!(day(json!({"time": "2026-10-27T08:00:00"})), "2026-10-27");
        assert_eq!(day(json!({"t": "2026-10-28"})), "2026-10-28");
        assert_eq!(
            day(json!({"ts": 0, "time": "2026-10-29T10:00"})),
            "2026-10-29",
            "a non-positive number is passed over"
        );
        assert_eq!(day(json!({"ts": "nonsense"})), "2030-01-01");
        assert_eq!(day(json!({"ts": 1e300})), "2030-01-01", "a time too large to be one is now, not a failed request");
        assert_eq!(day(json!({})), "2030-01-01");
        let z = parse_iso_local("2026-10-26T15:30:00Z").unwrap();
        assert_eq!(
            z,
            chrono::Utc.with_ymd_and_hms(2026, 10, 26, 15, 30, 0).unwrap().with_timezone(&Local).naive_local()
        );
    }

    #[test]
    fn a_blob_is_written_once_under_its_hash() {
        let dir = tempfile::tempdir().unwrap();
        let (sha, new) = put_blob(dir.path(), b"bytes").unwrap();
        assert!(new);
        assert_eq!(sha, sha256_hex(b"bytes"));
        assert_eq!(fs::read(blob_path(dir.path(), &sha)).unwrap(), b"bytes");
        assert_eq!(put_blob(dir.path(), b"bytes").unwrap(), (sha, false));
    }
}
