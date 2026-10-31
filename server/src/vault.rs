//! What the vault holds, and forgetting part of it (#12). The unit the user sees is a source (mail, calendar, phone,
//! PC, diary, address book) and a span of days. Forgetting a span takes together, so nothing of it is left to find:
//!   - its records (`raw/<kind>/<day>.ndjson`) and the notes made of them (`Mail/`, `Events/`, `Diary/`);
//!   - the kept files only those records named (a file another record still names stays), and their cached text;
//!   - the to-dos proposed from them and not taken yet (one the user took is theirs);
//!   - the index: its next update finds the files gone and forgets them.
//!
//! [`plan`] says what would go and deletes nothing; [`forget`] does it. Nothing else in the server deletes records.
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;
use serde_json::Value;

use crate::config::Paths;

static DAY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{4}-\d{2}-\d{2})").unwrap());

/// One source as the user knows it.
struct Source {
    id: &'static str,
    label: &'static str,
    /// record folders under raw/
    raw: &'static [&'static str],
    /// note folders of the vault
    notes: &'static [&'static str],
    /// record folders whose lines name kept files (`sha256`)
    sightings: &'static [&'static str],
    /// the `source` of the to-dos proposed from it
    todos: &'static [&'static str],
    /// read again whole from where it comes from on every sync (a calendar, an address book): what is forgotten
    /// here comes back while it is still there
    comes_back: bool,
}

const SOURCES: [Source; 6] = [
    Source {
        id: "mail",
        label: "메일과 첨부",
        raw: &["mailfile"],
        notes: &["Mail"],
        sightings: &["mailfile"],
        todos: &["메일"],
        comes_back: false,
    },
    Source {
        id: "calendar",
        label: "일정",
        raw: &[],
        notes: &["Events"],
        sightings: &[],
        todos: &["일정"],
        comes_back: true,
    },
    Source {
        id: "phone",
        label: "폰 기록",
        raw: &["lorelog"],
        notes: &[],
        sightings: &[],
        todos: &["통화", "메신저", "음성"],
        comes_back: false,
    },
    Source {
        id: "pc",
        label: "PC 기록과 파일",
        raw: &["pc", "pcfile"],
        notes: &[],
        sightings: &["pcfile"],
        todos: &[],
        comes_back: false,
    },
    Source {
        id: "diary",
        label: "일기와 대화",
        raw: &["diary", "corrections"],
        notes: &["Diary"],
        sightings: &[],
        todos: &[],
        comes_back: false,
    },
    Source {
        id: "contacts",
        label: "주소록",
        raw: &["contacts"],
        notes: &[],
        sightings: &[],
        todos: &[],
        comes_back: true,
    },
];

/// Every folder a sighting of a kept file can be in (a file stays while any of them names it).
const ALL_SIGHTINGS: [&str; 2] = ["mailfile", "pcfile"];

/// What one source holds.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Held {
    pub source: String,
    pub label: String,
    pub files: usize,
    pub bytes: u64,
    /// the first and last day it holds (YYYY-MM-DD)
    pub first: String,
    pub last: String,
}

/// Days, both included.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct Span {
    pub from: String,
    pub to: String,
}

/// What forgetting takes (or took).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Plan {
    /// the files, as paths
    pub files: Vec<String>,
    pub bytes: u64,
    /// to-dos proposed from them, not taken
    pub todos: usize,
    /// the source sends it again on its next sync while it is still there (turn it off there, or delete it there)
    pub comes_back: bool,
}

fn source(id: &str) -> anyhow::Result<&'static Source> {
    SOURCES.iter().find(|s| s.id == id).ok_or_else(|| anyhow::anyhow!("no such source: {id}"))
}

fn checked(span: &Span) -> anyhow::Result<()> {
    let day = |d: &str| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok();
    if !day(&span.from) || !day(&span.to) || span.from > span.to {
        anyhow::bail!("days must be YYYY-MM-DD, from not after to");
    }
    Ok(())
}

/// Every regular file under `dir` (links are not followed).
fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        match e.file_type() {
            Ok(t) if t.is_dir() => walk(&e.path(), out),
            Ok(t) if t.is_file() => out.push(e.path()),
            _ => {}
        }
    }
}

/// The day a record file or note is of: its name (`2026-03-01.ndjson`, `2026-03-01-0900-x.md`), else the `date:` or
/// `start:` of a note's header.
fn day_of(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_string_lossy().to_string();
    if let Some(m) = DAY_RE.captures(&name) {
        return Some(m[1].to_string());
    }
    let head = crate::note::head(path, 4000);
    ["date", "start"]
        .iter()
        .find_map(|k| crate::note::field(&head, k).and_then(|v| DAY_RE.captures(v.trim()).map(|m| m[1].to_string())))
}

/// The record and note files of a source, with their days.
fn files_of(paths: &Paths, s: &Source) -> Vec<(PathBuf, String)> {
    let mut all = Vec::new();
    for r in s.raw {
        walk(&paths.raw().join(r), &mut all);
    }
    for n in s.notes {
        walk(&paths.vault.join(n), &mut all);
    }
    all.into_iter().filter_map(|p| day_of(&p).map(|d| (p, d))).collect()
}

/// What every source holds (a source with nothing is left out).
pub fn holdings(paths: &Paths) -> Vec<Held> {
    let blobs = kept_by(paths);
    SOURCES
        .iter()
        .filter_map(|s| {
            let files = files_of(paths, s);
            let mut held = Held {
                source: s.id.into(),
                label: s.label.into(),
                files: 0,
                bytes: 0,
                first: String::new(),
                last: String::new(),
            };
            for (p, d) in &files {
                held.files += 1;
                held.bytes += fs::metadata(p).map(|m| m.len()).unwrap_or(0);
                if held.first.is_empty() || *d < held.first {
                    held.first = d.clone();
                }
                if *d > held.last {
                    held.last = d.clone();
                }
            }
            // the kept files its records name
            for sha in blobs.iter().filter(|(_, by)| s.sightings.contains(&by.as_str())).map(|(sha, _)| sha) {
                if let Ok(m) = fs::metadata(crate::store::blob_path(&paths.data, sha)) {
                    held.files += 1;
                    held.bytes += m.len();
                }
            }
            (held.files > 0).then_some(held)
        })
        .collect()
}

/// (kept file, the sightings folder naming it), one per pair.
fn kept_by(paths: &Paths) -> BTreeSet<(String, String)> {
    let mut out = BTreeSet::new();
    for kind in ALL_SIGHTINGS {
        let mut files = Vec::new();
        walk(&paths.raw().join(kind), &mut files);
        for f in files {
            for sha in shas(&f) {
                out.insert((sha, kind.to_string()));
            }
        }
    }
    out
}

/// The kept files a sightings file names.
fn shas(file: &Path) -> HashSet<String> {
    let Ok(text) = fs::read_to_string(file) else { return HashSet::new() };
    text.lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter_map(|v| v["sha256"].as_str().map(str::to_string))
        .filter(|s| s.len() == 64 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        .collect()
}

/// What forgetting `span` of `source` would take: (files, to-do ids).
fn work(paths: &Paths, id: &str, span: &Span) -> anyhow::Result<(Vec<PathBuf>, Vec<String>)> {
    let s = source(id)?;
    checked(span)?;
    let inside = |d: &str| span.from.as_str() <= d && d <= span.to.as_str();
    let mut gone: Vec<PathBuf> = files_of(paths, s).into_iter().filter(|(_, d)| inside(d)).map(|(p, _)| p).collect();
    // kept files named by the sightings that go, and by none that stay
    let going: HashSet<&PathBuf> = gone.iter().collect();
    let (mut named, mut kept) = (HashSet::new(), HashSet::new());
    for kind in ALL_SIGHTINGS {
        let mut files = Vec::new();
        walk(&paths.raw().join(kind), &mut files);
        for f in files {
            let target = if going.contains(&f) { &mut named } else { &mut kept };
            target.extend(shas(&f));
        }
    }
    let mut blobs: Vec<String> = named.difference(&kept).cloned().collect();
    blobs.sort();
    for sha in &blobs {
        let p = crate::store::blob_path(&paths.data, sha);
        if p.is_file() {
            gone.push(p);
        }
        // the text read out of it, of any kind and version
        if let Ok(dirs) = fs::read_dir(paths.data.join("cache")) {
            for d in dirs.flatten() {
                if let Ok(cached) = fs::read_dir(d.path().join(&sha[..2])) {
                    gone.extend(
                        cached
                            .flatten()
                            .map(|c| c.path())
                            .filter(|c| c.file_name().is_some_and(|n| n.to_string_lossy().starts_with(sha.as_str()))),
                    );
                }
            }
        }
    }
    gone.sort();
    gone.dedup();
    let todos = crate::assistant::todos::load(paths)["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|t| {
                    t["status"] == "proposed"
                        && t["day"].as_str().is_some_and(inside)
                        && t["source"].as_str().is_some_and(|src| s.todos.contains(&src))
                })
                .filter_map(|t| t["id"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok((gone, todos))
}

fn plan_of(s: &Source, files: &[PathBuf], todos: &[String]) -> Plan {
    Plan {
        comes_back: s.comes_back,
        files: files.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        bytes: files.iter().map(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0)).sum(),
        todos: todos.len(),
    }
}

/// What forgetting `span` of `source` would take. Deletes nothing.
pub fn plan(paths: &Paths, source: &str, span: &Span) -> anyhow::Result<Plan> {
    let (files, todos) = work(paths, source, span)?;
    Ok(plan_of(self::source(source)?, &files, &todos))
}

/// Forget `span` of `source` (cannot be undone). -> what was taken
pub fn forget(paths: &Paths, source: &str, span: &Span) -> anyhow::Result<Plan> {
    let (files, todos) = work(paths, source, span)?;
    let done = plan_of(self::source(source)?, &files, &todos);
    for f in &files {
        match fs::remove_file(f) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    if !todos.is_empty() {
        crate::assistant::todos::remove(paths, &todos)?;
    }
    log::info!("vault: forgot {source} {}..{}: {} files, {} to-dos", span.from, span.to, files.len(), todos.len());
    Ok(done)
}
