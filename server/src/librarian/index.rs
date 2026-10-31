//! The librarian's index: SQLite FTS5 over every document, kept in step with the files it came from.
//!
//! Korean is indexed in two-letter pieces ("거래처랑" -> 거래 래처 처랑) so a word is found inside a longer word with a
//! particle attached, and a two-letter word like "견적" is found at all; FTS5's own tokenizers do neither. A query word
//! becomes a phrase of its pieces (the pieces must be adjacent); a Korean query word ending in a particle also tries the
//! word without it, a single Korean letter becomes a prefix, and an English word of three letters or more matches longer
//! words that start with it. The words of a query are joined with OR and ranked by bm25: recall first, the best matches
//! on top.
//!
//! Only what changed is read again. Phone files need care: whether a day's voice is the user's depends on conversation
//! states that can arrive in another day's file. The index keeps every conversation state it has read (table convs,
//! until its file is removed; a phone file that cannot be read again keeps the states last read from it, since the raw
//! files only grow); when a state changes, the days that conversation spans are read again, and nothing else (a span
//! that cannot be believed makes every phone file be read again). When the allowed senders change, every phone file is
//! read again. Phone files are read in two passes, so memory holds one file at a time.
//!
//! Every update takes the write lock first (BEGIN IMMEDIATE) and only then lists the files and reads the allowed
//! senders, so `life sync` and a chat request never work from the same stale picture. A search reads one snapshot of
//! the index. An index of another version, or one no update has filled yet, is not searched ([`Outdated`]); the next
//! update empties and fills it in one transaction. A file that cannot be read (or holds a record that cannot be
//! understood) is skipped and its old documents are dropped; the next update tries it again. The index is a copy built
//! from the records: `reset` empties it, `update` builds it again.
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};
use serde_json::{Map, Value, json};
use unicode_normalization::UnicodeNormalization;

use super::records::{self, Doc, Kind, Span};
use crate::assistant::context::{Conversations, conversation_key, conversations_in, read_records};
use crate::assistant::senders::load_allow;
use crate::config::Paths;
use crate::fsutil::set_mode;

static WORD_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[0-9a-z]+|[가-힣\x{4e00}-\x{9fff}]+").unwrap());
static SPACES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

/// Particles and endings a spoken question puts after a word ("견적서는", "여수항만에서"); longest first.
static PARTICLES: LazyLock<Vec<&'static str>> = LazyLock::new(|| {
    let mut v = vec![
        "은",
        "는",
        "이",
        "가",
        "을",
        "를",
        "의",
        "에",
        "에서",
        "에게",
        "께",
        "한테",
        "로",
        "으로",
        "와",
        "과",
        "랑",
        "이랑",
        "하고",
        "도",
        "만",
        "까지",
        "부터",
        "처럼",
        "보다",
        "이나",
        "나",
        "요",
        "야",
        "란",
        "이란",
        "라는",
        "이라는",
        "이랑은",
        "에서는",
        "에는",
        "으로는",
        "에도",
        "에서도",
        "으로도",
        "로도",
        "한테도",
        "에게도",
    ];
    v.sort_by_key(|p| std::cmp::Reverse(p.chars().count()));
    v
});

pub const MAX_QUERY_WORDS: usize = 16;
/// The user's own words (often past questions with the same words) take this many slots at most.
pub const MAX_WORD_HITS: usize = 2;
const SNIPPET_BEFORE: usize = 60;
const SNIPPET_AFTER: usize = 140;
/// Matches looked at per search before the mails of one thread are folded together.
const WINDOW: i64 = 400;
/// Bump when the tables or what goes in them change: an index of another version is built again. (6 was the last
/// Python version; the Rust index is built anew once.)
pub const SCHEMA_VERSION: &str = "8"; // 8: who each record is with (#22)

/// A mail sent to more people than this is a mass mail: not counted as with any one of them.
pub const MASS_MAIL: i64 = 10;

const TABLES: &str = "
create table if not exists meta(key text primary key, value text);
create table if not exists files(path text primary key, kind text, stamp text);
create table if not exists docs(n integer primary key, file text, source text, kind text, day text, title text, thread text, text text);
create index if not exists docs_file on docs(file);
create index if not exists docs_thread on docs(thread);
create table if not exists convs(file text, seq integer, id text, data text);
create index if not exists convs_file on convs(file);
create virtual table if not exists fts using fts5(terms);
create table if not exists people(n integer, role text, name text, key text, addr text);
create index if not exists people_n on people(n);
create index if not exists people_addr on people(addr);
";

/// The index is of another version, or no update has finished yet: it is not searched, because it may hold what the
/// current rules leave out, or nothing at all.
#[derive(Debug)]
pub struct Outdated(pub String);

impl std::fmt::Display for Outdated {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for Outdated {}

// ---------- words ----------

/// The text as it is matched: composed Hangul (NFC; a name sent from a Mac often arrives decomposed), fullwidth forms
/// as plain ones, lower case. Only for matching: what is shown keeps the letters as written.
pub fn fold(text: &str) -> String {
    text.nfc().map(fold_char).collect::<String>().to_lowercase()
}

/// Fullwidth letters, digits and signs (ＡＢＣ, ２０２４) as their plain forms; one letter for one.
fn fold_char(c: char) -> char {
    let o = c as u32;
    if (0xFF01..=0xFF5E).contains(&o) { char::from_u32(o - 0xFEE0).unwrap_or(c) } else { c }
}

fn pieces(word: &str) -> Vec<String> {
    let chars: Vec<char> = word.chars().collect();
    if word.is_ascii() || chars.len() == 1 {
        return vec![word.to_string()];
    }
    chars.windows(2).map(|w| w.iter().collect()).collect()
}

/// The word without a trailing particle, if it has one and keeps two letters.
fn stem(word: &str) -> Option<String> {
    if word.is_ascii() {
        return None;
    }
    let n = word.chars().count();
    PARTICLES
        .iter()
        .find(|p| word.ends_with(**p) && n - p.chars().count() >= 2)
        .map(|p| word[..word.len() - p.len()].to_string())
}

/// The index terms of a text, space separated.
pub fn terms(text: &str) -> String {
    WORD_RE.find_iter(&fold(text)).flat_map(|m| pieces(m.as_str())).collect::<Vec<_>>().join(" ")
}

fn phrase(w: &str) -> String {
    if w.is_ascii() {
        return if w.len() >= 3 && !w.bytes().all(|b| b.is_ascii_digit()) {
            format!("\"{w}\"*")
        } else {
            format!("\"{w}\"")
        };
    }
    if w.chars().count() == 1 { format!("\"{w}\"*") } else { format!("\"{}\"", pieces(w).join(" ")) }
}

fn dedup(v: impl IntoIterator<Item = String>) -> Vec<String> {
    let mut seen = HashSet::new();
    v.into_iter().filter(|x| seen.insert(x.clone())).collect()
}

/// An FTS5 expression for the words of q (the first [`MAX_QUERY_WORDS`] of them, each with its stem), or "" if q has
/// none. Every word is quoted, so nothing in q can act as an FTS5 operator or column filter.
pub fn to_query(q: &str) -> String {
    let words: Vec<String> =
        dedup(WORD_RE.find_iter(&fold(q)).map(|m| m.as_str().to_string())).into_iter().take(MAX_QUERY_WORDS).collect();
    let parts = words.iter().map(|w| phrase(w)).chain(words.iter().filter_map(|w| stem(w)).map(|s| phrase(&s)));
    dedup(parts).join(" OR ")
}

/// A name as compared: folded, lower case, without spaces and dots (`김영희 수석`, `김 영희` → `김영희수석`, `김영희`).
pub fn name_key(name: &str) -> String {
    fold(name).to_lowercase().chars().filter(|c| !c.is_whitespace() && !matches!(c, '.' | '·' | '"' | '\'')).collect()
}

/// Someone the records are with, by address.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Someone {
    pub addr: String,
    /// the names they were written with, most used first
    pub names: Vec<String>,
    /// records with them (mass mail not counted)
    pub count: i64,
    /// the day of the latest
    pub last: String,
}

/// The part of the text around the first word of the query (or its stem) that appears in it.
pub fn snippet(text: &str, q: &str) -> String {
    let text: Vec<char> = text.nfc().collect(); // looks the same; lets the positions below match the shown text
    let low: Vec<char> = text.iter().map(|c| fold_char(*c).to_lowercase().next().unwrap_or(*c)).collect();
    let words: Vec<Vec<char>> = WORD_RE
        .find_iter(&fold(q))
        .flat_map(|m| [Some(m.as_str().to_string()), stem(m.as_str())])
        .flatten()
        .map(|w| w.chars().collect())
        .collect();
    let at = words.iter().filter_map(|w| find_chars(&low, w)).min();
    let start = at.map(|i| i.saturating_sub(SNIPPET_BEFORE)).unwrap_or(0);
    let end = start + SNIPPET_BEFORE + SNIPPET_AFTER;
    let piece: String = text[start.min(text.len())..end.min(text.len())].iter().collect();
    let body = SPACES.replace_all(&piece, " ").trim().to_string();
    format!("{}{body}{}", if start > 0 { "…" } else { "" }, if end < text.len() { "…" } else { "" })
}

fn find_chars(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

fn stamp(path: &Path) -> std::io::Result<String> {
    let m = fs::metadata(path)?;
    let ns = m.modified()?.duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    Ok(format!("{ns}:{}", m.len()))
}

/// The conversation records of a phone file, without decoding its other lines (most of a phone file is screen and app).
fn conversations_of(path: &Path) -> std::io::Result<Vec<Map<String, Value>>> {
    Ok(conversations_in(read_records(path, Some("\"conversation\""))?))
}

fn day_file(path: &str) -> String {
    Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

// ---------- the index ----------

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct Report {
    pub indexed: usize,
    pub removed: usize,
    pub skipped: usize,
    pub docs: usize,
}

/// What a search may leave out of one day: all of it, or only these lines the user said (as the chat log holds them).
pub enum Exclude<'a> {
    Day(&'a str),
    Words(&'a str, &'a [String]),
}

/// Where an update reads the allowed senders from.
pub type AllowSource = Box<dyn FnMut(&Paths) -> BTreeSet<String> + Send>;

pub struct Librarian {
    pub path: PathBuf,
    paths: Paths,
    db: Connection,
    /// Where the allowed senders come from: the user's list. A seam: a test changes it during an update.
    pub allowed: AllowSource,
    /// Called once after a search made its first hit. A seam: a test lands another update there.
    pub after_first_hit: Option<Box<dyn FnMut() + Send>>,
}

pub fn default_path(paths: &Paths) -> PathBuf {
    paths.data.join("librarian.sqlite")
}

fn err(e: rusqlite::Error) -> anyhow::Error {
    anyhow::Error::new(e)
}

impl Librarian {
    /// Open (and on first use, create) the index. `timeout`: how long to wait for another process holding the lock.
    pub fn open(paths: &Paths, path: Option<PathBuf>, timeout: Duration) -> anyhow::Result<Librarian> {
        let path = path.unwrap_or_else(|| default_path(paths));
        let dir = path.parent().unwrap_or(Path::new("."));
        if !dir.exists() {
            fs::create_dir_all(dir)?;
            set_mode(dir, 0o700); // like the rest of the data directory
        }
        if !path.exists() {
            // create it private first so the WAL files SQLite adds inherit the mode
            crate::fsutil::open_append_private(&path)?;
        }
        let db = Connection::open(&path)?;
        db.busy_timeout(timeout)?;
        let _: String = db.query_row("pragma journal_mode=wal", [], |r| r.get(0))?;
        let mut lib =
            Librarian { path, paths: paths.clone(), db, allowed: Box::new(load_allow), after_first_hit: None };
        if lib.version().is_none() {
            lib.rebuild_tables(true)?; // a new file. An index of another version is left as it is until update()
        }
        Ok(lib)
    }

    fn version(&self) -> Option<String> {
        self.meta("schema")
    }

    fn meta(&self, key: &str) -> Option<String> {
        self.db.query_row("select value from meta where key = ?", [key], |r| r.get(0)).optional().ok().flatten()
    }

    /// Drop every table and make them again; the caller holds the write lock.
    fn make_tables(&self) -> rusqlite::Result<()> {
        for t in ["docs", "files", "convs", "fts", "meta"] {
            self.db.execute_batch(&format!("drop table if exists {t}"))?;
        }
        self.db.execute_batch(TABLES)?;
        self.db.execute("insert into meta(key, value) values ('schema', ?)", [SCHEMA_VERSION])?;
        Ok(())
    }

    fn in_write<T>(&mut self, f: impl FnOnce(&mut Self) -> anyhow::Result<T>) -> anyhow::Result<T> {
        self.db.execute_batch("begin immediate")?;
        match f(self) {
            Ok(v) => {
                self.db.execute_batch("commit")?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.db.execute_batch("rollback");
                Err(e)
            }
        }
    }

    fn rebuild_tables(&mut self, only_if_new: bool) -> anyhow::Result<()> {
        self.in_write(|me| {
            if only_if_new && me.version().is_some() {
                return Ok(()); // another process made them while we waited
            }
            me.make_tables().map_err(err)
        })
    }

    /// Empty the index (the file stays, so open connections keep working); the next update builds it again. To empty
    /// and fill it without a moment of an empty index in between, use `update(true)`.
    pub fn reset(&mut self) -> anyhow::Result<()> {
        self.rebuild_tables(false)
    }

    fn forget(&self, path: &str) -> rusqlite::Result<()> {
        self.db.execute("delete from fts where rowid in (select n from docs where file = ?)", [path])?;
        self.db.execute("delete from people where n in (select n from docs where file = ?)", [path])?;
        self.db.execute("delete from docs where file = ?", [path])?;
        Ok(())
    }

    fn add(&self, path: &str, d: &Doc) -> rusqlite::Result<()> {
        self.db.execute(
            "insert into docs(file, source, kind, day, title, thread, text) values (?, ?, ?, ?, ?, ?, ?)",
            params![path, d.source, d.kind, d.day, d.title, d.thread, d.text],
        )?;
        let n = self.db.last_insert_rowid();
        self.db.execute(
            "insert into fts(rowid, terms) values (?, ?)",
            params![n, terms(&format!("{}\n{}\n{}", d.title, d.extra, d.text))],
        )?;
        let mut st = self.db.prepare_cached("insert into people(n, role, name, key, addr) values (?, ?, ?, ?, ?)")?;
        for p in &d.people {
            st.execute(params![n, p.role, p.name, name_key(&p.name), p.addr])?;
        }
        Ok(())
    }

    fn keep(&self, path: &str, kind: Kind, stamp: &str, docs: &[Doc]) -> rusqlite::Result<()> {
        self.forget(path)?;
        for d in docs {
            self.add(path, d)?;
        }
        self.db.execute(
            "insert or replace into files(path, kind, stamp) values (?, ?, ?)",
            params![path, kind.name(), stamp],
        )?;
        Ok(())
    }

    fn skip(&self, path: &str, why: &dyn std::fmt::Display) -> rusqlite::Result<()> {
        self.forget(path)?;
        self.db.execute("delete from files where path = ?", [path])?; // read it again next time
        log::warn!(
            "librarian: skipped {}: {why}",
            Path::new(path).file_name().map(|n| n.to_string_lossy()).unwrap_or_default()
        );
        Ok(())
    }

    /// The latest state of every conversation read so far (a later file, then a later line, wins).
    fn convs(&self) -> rusqlite::Result<Conversations> {
        let mut out = Conversations::new();
        let mut st = self.db.prepare("select id, data from convs order by file, seq")?;
        let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (id, data) = row?;
            if let Ok(Value::Object(o)) = serde_json::from_str::<Value>(&data) {
                out.insert(id, o);
            }
        }
        Ok(out)
    }

    /// (files to read again, files gone). Every phone file is read again when the allowed senders changed.
    fn changes(
        &self,
        found: &mut Vec<(String, Kind, PathBuf)>,
        policy: &str,
    ) -> anyhow::Result<(BTreeSet<String>, Vec<String>)> {
        let known: HashMap<String, String> = {
            let mut st = self.db.prepare("select path, stamp from files")?;
            st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        let mut dirty = BTreeSet::new();
        found.retain(|(path, _, p)| match stamp(p) {
            Ok(s) => {
                if known.get(path) != Some(&s) {
                    dirty.insert(path.clone());
                }
                true
            }
            Err(_) => false, // removed between listing and now (a sync is writing): treat it as gone
        });
        let listed: HashSet<&String> = found.iter().map(|f| &f.0).collect();
        let removed = known.keys().filter(|p| !listed.contains(p)).cloned().collect();
        if self.meta("policy").as_deref() != Some(policy) {
            dirty.extend(found.iter().filter(|f| f.1 == Kind::Lorelog).map(|f| f.0.clone()));
        }
        Ok((dirty, removed))
    }

    /// Keep the conversation states of the changed phone files, read again the days whose conversations changed, and
    /// index them all, one file at a time. -> (indexed, skipped)
    fn update_phone(
        &self,
        found: &[(String, Kind, PathBuf)],
        dirty: &BTreeSet<String>,
        allow: &BTreeSet<String>,
    ) -> anyhow::Result<(usize, usize)> {
        let phone: HashMap<&String, &PathBuf> =
            found.iter().filter(|f| f.1 == Kind::Lorelog).map(|f| (&f.0, &f.2)).collect();
        let todo: BTreeSet<String> = dirty.iter().filter(|p| phone.contains_key(p)).cloned().collect();
        // files whose conversation states are kept but that are gone (also one skipped before it went, which has no
        // files row any more)
        let kept: Vec<String> = {
            let mut st = self.db.prepare("select distinct file from convs")?;
            st.query_map([], |r| r.get::<_, String>(0))?.collect::<rusqlite::Result<_>>()?
        };
        let gone: Vec<String> = kept.into_iter().filter(|p| !phone.contains_key(p)).collect();
        if todo.is_empty() && gone.is_empty() {
            return Ok((0, 0));
        }
        let before = self.convs()?;
        let mut stamps: std::collections::BTreeMap<String, String> = Default::default();
        let mut skipped = 0;
        // first pass: stamps and conversation states only
        let mut take =
            |paths: &BTreeSet<String>, stamps: &mut std::collections::BTreeMap<String, String>| -> anyhow::Result<()> {
                for path in paths {
                    let p = phone[path];
                    match stamp(p).and_then(|s| conversations_of(p).map(|c| (s, c))) {
                        Ok((s, convs)) => {
                            stamps.insert(path.clone(), s);
                            self.db.execute("delete from convs where file = ?", [path])?;
                            for (i, c) in convs.iter().enumerate() {
                                self.db.execute(
                                    "insert into convs(file, seq, id, data) values (?, ?, ?, ?)",
                                    params![path, i as i64, conversation_key(c), Value::Object(c.clone()).to_string()],
                                )?;
                            }
                        }
                        Err(e) => {
                            // its conversation states stay as last read: the raw files only grow, so they are still the best
                            // we know, and dropping them could bring back an older state (a voice the user took back)
                            self.skip(path, &e.kind())?;
                            skipped += 1;
                        }
                    }
                }
                Ok(())
            };
        for path in &gone {
            self.db.execute("delete from convs where file = ?", [path])?;
        }
        take(&todo, &mut stamps)?;
        let after = self.convs()?;
        // a conversation whose state changed (or appeared, or went) decides again about the voice on its days
        let mut days = BTreeSet::new();
        let mut every_day = false;
        let ids: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
        for id in ids {
            if before.get(id) != after.get(id) {
                for c in [before.get(id), after.get(id)].into_iter().flatten() {
                    match records::conversation_days(c) {
                        Span::Unlistable => every_day = true,
                        Span::Days(d) => days.extend(d),
                        Span::Nothing => {}
                    }
                }
            }
        }
        let by_day: HashMap<String, &String> = phone.keys().map(|p| (day_file(p), *p)).collect();
        let wanted: BTreeSet<String> = if every_day {
            phone.keys().map(|p| (*p).clone()).collect()
        } else {
            days.iter().filter_map(|d| by_day.get(d)).map(|p| (*p).clone()).collect()
        };
        let more: BTreeSet<String> =
            wanted.into_iter().filter(|p| !stamps.contains_key(p) && !todo.contains(p)).collect();
        let after = if more.is_empty() {
            after
        } else {
            take(&more, &mut stamps)?;
            self.convs()?
        };
        let mut indexed = 0;
        for (path, s) in &stamps {
            // second pass: one file at a time
            match read_records(phone[path], None) {
                Ok(lines) => {
                    let docs = records::lorelog_docs(&day_file(path), &lines, &after, allow);
                    self.keep(path, Kind::Lorelog, s, &docs)?;
                    indexed += 1;
                }
                Err(e) => {
                    self.skip(path, &e.kind())?; // its conversation states, just read, stay
                    skipped += 1;
                }
            }
        }
        Ok((indexed, skipped))
    }

    /// Index new and changed files, forget removed ones. With `reset`, or when the index is of another version, it is
    /// emptied first in the same transaction, so a search meanwhile sees the old index, never an empty one.
    pub fn update(&mut self, reset: bool) -> anyhow::Result<Report> {
        let (indexed, skipped, removed) = self.in_write(|me| {
            let mut found: Vec<(String, Kind, PathBuf)> = records::sources(&me.paths)
                .into_iter()
                .map(|(k, p)| (p.to_string_lossy().into_owned(), k, p))
                .collect();
            let allow = (me.allowed)(&me.paths); // read once: the documents are made with it and it is what we remember
            let policy = records::policy_stamp(&allow);
            if reset || me.version().as_deref() != Some(SCHEMA_VERSION) {
                me.make_tables()?;
            }
            let (dirty, removed) = me.changes(&mut found, &policy)?;
            for path in &removed {
                me.forget(path)?;
                me.db.execute("delete from files where path = ?", [path])?;
            }
            let (mut indexed, mut skipped) = me.update_phone(&found, &dirty, &allow)?;
            for (path, kind, p) in &found {
                if *kind == Kind::Lorelog || !dirty.contains(path) {
                    continue;
                }
                match stamp(p).map_err(anyhow::Error::from).and_then(|s| records::docs(*kind, p).map(|d| (s, d))) {
                    Ok((s, docs)) => {
                        me.keep(path, *kind, &s, &docs)?;
                        indexed += 1;
                    }
                    Err(e) => {
                        me.skip(path, &e)?;
                        skipped += 1;
                    }
                }
            }
            me.db.execute("insert or replace into meta(key, value) values ('policy', ?)", [&policy])?;
            Ok((indexed, skipped, removed.len()))
        })?;
        let docs: i64 = self.db.query_row("select count(*) from docs", [], |r| r.get(0))?;
        Ok(Report { indexed, removed, skipped, docs: docs as usize })
    }

    fn thread_span(&self, thread: &str) -> rusqlite::Result<(i64, Option<String>, Option<String>)> {
        self.db.query_row("select count(*), min(day), max(day) from docs where thread = ?", [thread], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?))
        })
    }

    /// Documents matching the words of q, best first. The mails of one thread come back as one hit (its best-matching
    /// mail) with the thread's size and the days it spans; the user's own words take at most [`MAX_WORD_HITS`] slots
    /// unless nothing else is found. The search reads one snapshot of the index.
    /// -> `[{source, kind, day, title, snippet, ref, count, first, last}]`
    pub fn search(&mut self, q: &str, limit: usize, exclude: Option<Exclude>) -> anyhow::Result<Vec<Value>> {
        let expr = to_query(q);
        if expr.is_empty() {
            return Ok(Vec::new());
        }
        self.db.execute_batch("begin")?; // one read snapshot (WAL) for every query below
        let result = (|| {
            if self.version().as_deref() != Some(SCHEMA_VERSION) {
                return Err(Outdated(format!(
                    "index version {:?}, expected {SCHEMA_VERSION}: run update",
                    self.version()
                ))
                .into());
            }
            if self.meta("policy").is_none() {
                return Err(Outdated("index not built yet: run update".into()).into()); // empty, but not because nothing is there
            }
            self.search_in(&expr, q, limit, exclude)
        })();
        let _ = self.db.execute_batch("rollback"); // nothing was written; ends the snapshot
        result
    }

    fn search_in(&mut self, expr: &str, q: &str, limit: usize, exclude: Option<Exclude>) -> anyhow::Result<Vec<Value>> {
        let mut sql =
            "select d.n, d.thread, d.source from fts join docs d on d.n = fts.rowid where fts match ?".to_string();
        let mut args: Vec<String> = vec![expr.to_string()];
        match exclude {
            Some(Exclude::Day(day)) => {
                sql += " and d.day != ?";
                args.push(day.into());
            }
            Some(Exclude::Words(day, said)) => {
                // compared exactly: the documents hold these lines as said, stripped, or as a correction
                let texts: BTreeSet<String> =
                    said.iter().flat_map(|w| [w.clone(), w.trim().to_string(), records::word_text(w)]).collect();
                if !texts.is_empty() {
                    sql += &format!(
                        " and not (d.day = ? and d.source = 'word' and d.text in ({}))",
                        vec!["?"; texts.len()].join(",")
                    );
                    args.push(day.into());
                    args.extend(texts);
                }
            }
            None => {}
        }
        sql += &format!(" order by fts.rank limit {WINDOW}");
        let rows: Vec<(i64, Option<String>, String)> = {
            let mut st = self.db.prepare(&sql)?;
            st.query_map(params_from_iter(args.iter()), |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        // thread (or the document itself) -> its best-ranked document, in rank order
        let mut seen = HashSet::new();
        let best: Vec<&(i64, Option<String>, String)> = rows
            .iter()
            .filter(|(n, thread, _)| seen.insert(thread.clone().unwrap_or_else(|| format!("#{n}"))))
            .collect();
        let (mut chosen, mut spill, mut words) = (Vec::new(), Vec::new(), 0);
        for g in best {
            if chosen.len() >= limit {
                break;
            }
            if g.2 == "word" {
                if words >= MAX_WORD_HITS {
                    spill.push(g);
                    continue;
                }
                words += 1;
            }
            chosen.push(g);
        }
        let room = limit.saturating_sub(chosen.len());
        chosen.extend(spill.into_iter().take(room));
        let mut out = Vec::new();
        for (n, thread, _) in chosen {
            out.push(self.hit(*n, thread.as_deref(), q)?);
            if let Some(hook) = self.after_first_hit.as_mut() {
                hook();
                self.after_first_hit = None;
            }
        }
        Ok(out)
    }

    /// One hit as the assistant gets it: `{source, kind, day, title, snippet, ref, count, first, last}`.
    fn hit(&self, n: i64, thread: Option<&str>, q: &str) -> anyhow::Result<Value> {
        let (source, kind, day, title, text, file): (String, String, String, String, String, String) =
            self.db.query_row("select source, kind, day, title, text, file from docs where n = ?", [n], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?))
            })?;
        let (count, first, last) = match thread {
            Some(t) => self.thread_span(t)?,
            None => (1, Some(day.clone()), Some(day.clone())),
        };
        let first = first.filter(|f| !f.is_empty()).unwrap_or_else(|| day.clone());
        let last = last.filter(|f| !f.is_empty()).unwrap_or_else(|| day.clone());
        Ok(
            json!({"source": source, "kind": kind, "day": day, "title": title, "snippet": snippet(&text, q), "ref": file,
                  "count": count, "first": first, "last": last}),
        )
    }

    fn ready(&self) -> anyhow::Result<()> {
        if self.version().as_deref() != Some(SCHEMA_VERSION) {
            return Err(
                Outdated(format!("index version {:?}, expected {SCHEMA_VERSION}: run update", self.version())).into()
            );
        }
        if self.meta("policy").is_none() {
            return Err(Outdated("index not built yet: run update".into()).into());
        }
        Ok(())
    }

    /// The people whose name contains `name` (compared as [`name_key`]), one per address: a name alone never makes
    /// two addresses one person. The user's own addresses (the senders of their sent mail) are left out. Most
    /// records first.
    pub fn people(&self, name: &str) -> anyhow::Result<Vec<Someone>> {
        self.ready()?;
        let key = name_key(name);
        if key.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let sql = format!(
            "with mine as (select distinct p.addr from people p join docs d on d.n = p.n where d.kind = '보낸 메일' and p.role = 'from'),
                  mass as (select n from people where role in ('to', 'cc') group by n having count(*) > {MASS_MAIL}),
                  who as (select distinct addr from people where instr(key, ?) > 0 and addr not in (select addr from mine))
             select p.addr, p.name, count(distinct p.n), max(d.day) from people p join docs d on d.n = p.n
             where p.addr in (select addr from who) and p.n not in (select n from mass)
             group by p.addr, p.name"
        );
        let rows: Vec<(String, String, i64, String)> = {
            let mut st = self.db.prepare(&sql)?;
            st.query_map([key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
                .collect::<rusqlite::Result<_>>()?
        };
        // address -> (names with how often each was used, records, latest day)
        type Seen = (Vec<(i64, String)>, i64, String);
        let mut by: BTreeMap<String, Seen> = BTreeMap::new();
        for (addr, name, count, last) in rows {
            let e = by.entry(addr).or_insert_with(|| (Vec::new(), 0, String::new()));
            if !name.is_empty() {
                e.0.push((count, name));
            }
            e.1 = e.1.max(count);
            if last > e.2 {
                e.2 = last;
            }
        }
        let mut out: Vec<Someone> = by
            .into_iter()
            .map(|(addr, (mut names, count, last))| {
                names.sort_by_key(|n| std::cmp::Reverse(n.0));
                Someone { addr, names: names.into_iter().map(|(_, n)| n).collect(), count, last }
            })
            .collect();
        out.sort_by_key(|s| std::cmp::Reverse(s.count));
        Ok(out)
    }

    /// What was with these people up to `until` (YYYY-MM-DD, that day included; later events are not "what was"):
    /// mail they sent or received and events they attended, newest first, then mail they were only copied on; mass
    /// mail left out; one hit per thread, and per title for events (a weekly meeting is one). `q` (if any) narrows it
    /// to records with its words.
    pub fn with_people(&mut self, addrs: &[String], q: &str, until: &str, limit: usize) -> anyhow::Result<Vec<Value>> {
        self.ready()?;
        if addrs.is_empty() {
            return Ok(Vec::new());
        }
        let marks = vec!["?"; addrs.len()].join(",");
        let expr = to_query(q);
        let narrow =
            if expr.is_empty() { String::new() } else { "and d.n in (select rowid from fts where fts match ?)".into() };
        let sql = format!(
            "with mass as (select n from people where role in ('to', 'cc') group by n having count(*) > {MASS_MAIL})
             select d.n, d.thread, max(p.role != 'cc') as direct, d.day, d.source, d.title from people p join docs d on d.n = p.n
             where p.addr in ({marks}) and d.n not in (select n from mass) and d.day <= ? {narrow}
             group by d.n order by direct desc, d.day desc, d.n desc"
        );
        let mut args: Vec<String> = addrs.to_vec();
        args.push(until.to_string());
        if !expr.is_empty() {
            args.push(expr);
        }
        self.db.execute_batch("begin")?;
        let result = (|| {
            let rows: Vec<(i64, Option<String>, String, String)> = {
                let mut st = self.db.prepare(&sql)?;
                st.query_map(params_from_iter(args.iter()), |r| Ok((r.get(0)?, r.get(1)?, r.get(4)?, r.get(5)?)))?
                    .collect::<rusqlite::Result<_>>()?
            };
            let mut seen = HashSet::new();
            let mut out = Vec::new();
            for (n, thread, source, title) in rows {
                let key = match (&thread, source.as_str()) {
                    (Some(t), _) => t.clone(),
                    (None, "event") => format!("event:{title}"),
                    (None, _) => format!("#{n}"),
                };
                if !seen.insert(key) {
                    continue;
                }
                out.push(self.hit(n, thread.as_deref(), q)?);
                if out.len() >= limit {
                    break;
                }
            }
            Ok(out)
        })();
        let _ = self.db.execute_batch("rollback");
        result
    }

    /// The thread an indexed file belongs to (a mail's `mail:<id>`); None for a file not indexed or not in a thread.
    pub fn thread_of(&self, file: &str) -> Option<String> {
        self.db
            .query_row("select thread from docs where file = ? and thread is not null limit 1", [file], |r| r.get(0))
            .ok()
    }

    /// How much is indexed; `outdated` for an index of another version or one never filled (nothing is counted then:
    /// its tables may not be these).
    pub fn stats(&self) -> Value {
        if self.version().as_deref() != Some(SCHEMA_VERSION) || self.meta("policy").is_none() {
            return json!({"docs": 0, "by_source": {}, "outdated": true, "files": 0});
        }
        let by: Vec<(String, i64)> = self
            .db
            .prepare("select source, count(*) from docs group by source order by source")
            .and_then(|mut st| st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect())
            .unwrap_or_default();
        let files: i64 = self.db.query_row("select count(*) from files", [], |r| r.get(0)).unwrap_or(0);
        let by_source: Map<String, Value> = by.iter().map(|(k, v)| (k.clone(), json!(v))).collect();
        json!({"docs": by.iter().map(|(_, v)| v).sum::<i64>(), "by_source": by_source, "outdated": false, "files": files})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn korean_is_indexed_in_two_letter_pieces() {
        assert_eq!(terms("거래처랑"), "거래 래처 처랑");
        assert_eq!(terms("PO-2024 Quote"), "po 2024 quote");
        assert_eq!(terms("책 좀"), "책 좀");
    }

    #[test]
    fn a_long_query_keeps_its_last_words_and_is_bounded() {
        let q = to_query(
            "지난달에 부산에서 거래처랑 만난 사람이 보낸 견적서는 메일로 언제 왔는지 회의록에도 있었나요 트렌드마이너",
        );
        assert!(q.contains("\"회의 의록\""), "{q}");
        assert!(q.contains("\"트렌 렌드 드마 마이 이너\""));
        let many: Vec<String> = (0..200).map(|i| format!("단어{i}")).collect();
        assert!(to_query(&many.join(" ")).matches(" OR ").count() < 16 * 2);
    }

    #[test]
    fn a_query_quotes_every_word() {
        assert_eq!(
            to_query("견적서는 NEAR broker 3"),
            "\"견적 적서 서는\" OR \"near\"* OR \"broker\"* OR \"3\" OR \"견적 적서\""
        );
        assert_eq!(to_query(" ... "), "");
    }

    #[test]
    fn decomposed_hangul_and_fullwidth_letters_are_matched_as_plain_ones() {
        let nfd: String = "견적서".nfd().collect();
        assert_eq!(terms(&nfd), "견적 적서");
        assert_eq!(terms("ＡＢＣ전자"), "abc 전자");
    }

    #[test]
    fn the_snippet_is_around_the_first_match_and_keeps_the_letters_as_written() {
        let text = format!("{}ＡＢＣ전각상사 회신", "앞".repeat(100));
        let s = snippet(&text, "전각상사");
        assert!(s.starts_with('…') && s.contains("ＡＢＣ전각상사"), "{s}");
    }
}
