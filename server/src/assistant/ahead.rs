//! The assistant speaks first, where the next thing happens (#60, decided there): ten minutes before an event, one
//! line about a request that someone of the event, or their organization, made and the user has not answered, with
//! the open to-dos of the same matter on the same line.
//!
//! Only what is certain, else nothing is said:
//!   - an attendee address that is exactly a person or an organization of the graph (`person:<addr>`, `org:<domain>`);
//!   - a request with the record it came from, the words that say it and its day ([`Librarian::open_requests_at`]);
//!   - the sentence is a template, not the model's.
//!
//! How little: one line per event, never the same (event, matter) again, nothing for all-day events, for events of many
//! people ([`CROWD`]) or for what
//! would be said between 22:00 and 07:00, nothing from an organization the user stopped (the phone stops one after
//! three of its lines were swiped away). There is no daily cap. What was said is kept in `<data>/ahead.json`, so the
//! phone (which asks two hours ahead and says it itself, the server on or not) and the desktop (which asks for what
//! starts within ten minutes) do not both say it. The phone's buttons are corrections of the request's relation:
//! 「이미 했어요」 is [`ANSWERED`], 「이건 아니에요」 is `-` (#10).
use std::collections::{BTreeMap, BTreeSet};
use std::sync::Mutex;
use std::time::Duration;

use chrono::{Datelike, NaiveDate, NaiveDateTime, TimeDelta, Timelike};
use serde_json::{Value, json};

use super::context::subj_key;
pub use super::corrections::ANSWERED;
use crate::config::Paths;
use crate::librarian::Librarian;
use crate::librarian::graph::Open;
use crate::librarian::index::MASS_MAIL;
use crate::librarian::records::{addresses, event_key, list_items, phone_event_attendees, phone_event_times};

/// how long before an event its line is said
pub const LEAD: TimeDelta = TimeDelta::minutes(10);
/// how far ahead the phone asks (it says the lines itself, at their time)
pub const WINDOW: TimeDelta = TimeDelta::minutes(120);
/// the most events one ask may carry
pub const MAX_EVENTS: usize = 100;
/// the open to-dos named on a line
const TODOS: usize = 2;
/// the most organizations and people kept stopped
const MAX_STOPPED: usize = 500;
/// the longest event title or label put in a line
const SHOWN: usize = 60;
/// an event with more addresses than this (the user's among them) is a team's or a company's: the requests of some of
/// its people are not what it is about, so nothing is said of it (as a mail to more than [`MASS_MAIL`] is a mass mail)
pub const CROWD: usize = MASS_MAIL as usize + 1;

/// Is an event with these attendee addresses one of many people ([`CROWD`])?
pub fn crowded(addrs: &[String]) -> bool {
    addrs.len() > CROWD
}

static LOCK: Mutex<()> = Mutex::new(());

/// An event coming up.
#[derive(Clone, Debug, PartialEq)]
pub struct Event {
    /// the asker's own key for it (handed back)
    pub key: String,
    /// local time
    pub start: NaiveDateTime,
    pub title: String,
    /// `addr` or `name <addr>`; anything else is no one certain
    pub attendees: Vec<String>,
    pub all_day: bool,
}

/// What a device asks: its events (the vault's are added) and the organizations or people it stopped.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ask {
    pub events: Vec<Event>,
    pub stopped: Vec<String>,
}

/// One line to say, at `at`.
#[derive(Clone, Debug, PartialEq)]
pub struct Ahead {
    pub event: String,
    pub start: NaiveDateTime,
    pub at: NaiveDateTime,
    pub title: String,
    pub text: String,
    /// what the chat opens with when the line is tapped
    pub question: String,
    /// `org:<domain>` or `person:<addr>`: what three swipes stop
    pub stop: String,
    /// `edge:<key>`: the ref of the buttons' corrections
    pub edge: String,
    /// the wrong side of those corrections
    pub wrong: String,
    pub matter: String,
}

impl Ahead {
    /// As the phone gets it: times in epoch milliseconds.
    pub fn json(&self) -> Value {
        let ms = |t: &NaiveDateTime| t.and_local_timezone(chrono::Local).earliest().map(|t| t.timestamp_millis());
        json!({"event": self.event, "start": ms(&self.start), "at": ms(&self.at), "title": self.title,
               "text": self.text, "question": self.question, "stop": self.stop, "edge": self.edge,
               "wrong": self.wrong, "matter": self.matter})
    }
}

/// Is this a time nothing is said at (22:00–07:00)?
pub fn quiet(t: NaiveDateTime) -> bool {
    !(7..22).contains(&t.hour())
}

/// The lines for the events starting after `now` and at most `within` from it (the asked ones and the vault's), and
/// kept as said. Each event at most once, and only with what is certain.
pub fn take(paths: &Paths, ask: &Ask, now: NaiveDateTime, within: TimeDelta) -> anyhow::Result<Vec<Ahead>> {
    let mut seen = BTreeSet::new();
    let mut events: Vec<Event> = ask
        .events
        .iter()
        .cloned()
        .chain(vault_events(paths, now, within))
        .filter(|e| !e.all_day && e.start > now && e.start <= now + within && !quiet(e.start - LEAD))
        .filter(|e| seen.insert(identity(e)))
        .collect();
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut said = Said::load(paths);
    let fresh: Vec<String> = ask.stopped.iter().filter(|s| stop_key(s)).cloned().collect();
    let mut changed = said.prune(now);
    for s in fresh {
        if said.stopped.len() < MAX_STOPPED || said.stopped.contains(&s) {
            changed |= said.stopped.insert(s);
        }
    }
    events.retain(|e| !said.events.contains_key(&identity(e)));
    let mut out = Vec::new();
    if !events.is_empty() {
        events.sort_by(|a, b| a.start.cmp(&b.start).then(a.key.cmp(&b.key)));
        let lib = Librarian::open(paths, None, Duration::from_secs(5))?;
        let todos = super::todos::load(paths);
        for e in &events {
            let addrs: Vec<String> =
                e.attendees.iter().flat_map(|a| addresses(a, "attendee")).map(|p| p.addr).collect();
            if addrs.is_empty() || crowded(&addrs) {
                continue;
            }
            let id = identity(e);
            let pick = lib
                .open_requests_at(&addrs)?
                .into_iter()
                .find(|o| !said.stopped.contains(&o.stop) && !said.pairs.contains_key(&format!("{id}|{}", o.case)));
            let Some(o) = pick else { continue };
            let open = open_todos(&todos, &lib.case_subjects(&o.case));
            let ms = e.start.and_utc().timestamp_millis();
            said.events.insert(id.clone(), ms);
            said.pairs.insert(format!("{id}|{}", o.case), ms);
            changed = true;
            out.push(line(e, &o, &open, now));
        }
    }
    if changed {
        said.save(paths)?;
    }
    Ok(out)
}

/// The line, from a template.
fn line(e: &Event, o: &Open, todos: &[String], now: NaiveDateTime) -> Ahead {
    let mins = ((e.start - now).num_seconds() + 59) / 60;
    let title = format!("{}분 뒤 「{}」", mins.clamp(1, LEAD.num_minutes()), shown(&e.title));
    let Unanswered { mut text, question, wrong } = Unanswered::of(o);
    if !todos.is_empty() {
        text.push_str(&format!(" 할 일: {}", todos.join(", ")));
    }
    Ahead {
        event: e.key.clone(),
        start: e.start,
        at: e.start - LEAD,
        title,
        text,
        question,
        stop: o.stop.clone(),
        edge: o.edge.clone(),
        wrong,
        matter: o.case.clone(),
    }
}

/// How a request not answered is said (#60; the schedule's notes say it so too, #62).
pub(super) struct Unanswered {
    /// `이담당님이 3월 25일에 요청한 「설치 사진」에 아직 답하지 않았어요.`
    pub text: String,
    /// what the chat opens with
    pub question: String,
    /// the wrong side of a correction of it
    pub wrong: String,
}

impl Unanswered {
    pub fn of(o: &Open) -> Unanswered {
        let who =
            if o.who.starts_with("org:") { format!("{}에서", o.who_name) } else { format!("{}님이", o.who_name) };
        let label = shown(&o.label);
        let day = NaiveDate::parse_from_str(&o.since, "%Y-%m-%d")
            .map(|d| format!("{}월 {}일", d.month(), d.day()))
            .unwrap_or_else(|_| o.since.clone());
        Unanswered {
            text: format!("{who} {day}에 요청한 「{label}」에 아직 답하지 않았어요."),
            question: format!("{who} 요청한 「{label}」 건 어떻게 됐지?"),
            wrong: format!("{who} {}에 요청한 「{label}」에 아직 답하지 않았다", o.since),
        }
    }
}

/// One line, at most [`SHOWN`] characters, without the quote marks the line puts around it.
pub(super) fn shown(s: &str) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ").replace(['「', '」'], "");
    let one = if one.is_empty() { "(제목 없음)".to_string() } else { one };
    one.chars().take(SHOWN).collect()
}

/// The open to-dos of the matter: those of a mail thread with one of its subjects.
pub(super) fn open_todos(todos: &Value, subjects: &[String]) -> Vec<String> {
    let keys: BTreeSet<String> = subjects.iter().map(|s| subj_key(s)).filter(|k| !k.is_empty()).collect();
    todos["items"]
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|i| i["status"] == super::todos::OPEN)
                .filter(|i| i["thread"].as_str().is_some_and(|t| keys.contains(t)))
                .filter_map(|i| i["title"].as_str().map(shown))
                .take(TODOS)
                .collect()
        })
        .unwrap_or_default()
}

/// The same event from any device: its start to the minute and its title's letters (as the index counts it once).
fn identity(e: &Event) -> String {
    event_key(&e.start.format("%Y-%m-%dT%H:%M").to_string(), &e.title)
}

fn stop_key(s: &str) -> bool {
    s.len() <= 200 && (s.starts_with("org:") || s.starts_with("person:")) && !s.contains(char::is_whitespace)
}

/// The vault's events starting in the window: CalDAV's `Events/` notes, then the phone's calendar as the index has it
/// (#77; an index not built yet has none).
fn vault_events(paths: &Paths, now: NaiveDateTime, within: TimeDelta) -> Vec<Event> {
    let last = now + within;
    let mut out = note_events(paths, now, last);
    let phone = Librarian::open(paths, None, Duration::from_secs(5)).and_then(|lib| lib.phone_calendar(now, last));
    for (key, rec) in phone.unwrap_or_default() {
        let Some((start, _, all_day)) = phone_event_times(&rec) else { continue };
        out.push(Event {
            key,
            start,
            title: rec.get("title").and_then(Value::as_str).unwrap_or("").to_string(),
            attendees: phone_event_attendees(&rec),
            all_day,
        });
    }
    out
}

/// CalDAV's `Events/` notes starting from `now` to `last`.
fn note_events(paths: &Paths, now: NaiveDateTime, last: NaiveDateTime) -> Vec<Event> {
    let mut days = Vec::new();
    let mut d = now.date();
    while d <= last.date() {
        days.push(d.format("%Y-%m-%d").to_string());
        d += TimeDelta::days(1);
    }
    let mut out = Vec::new();
    for day in days {
        for p in super::util::files_like(&paths.vault.join("Events"), &day, ".md") {
            let Ok(text) = crate::note::read(&p) else { continue };
            let (front, _) = crate::note::split(&text);
            let get = |k: &str| {
                crate::note::field(front, k).unwrap_or_default().trim_matches('"').trim_matches('\'').to_string()
            };
            let start = get("start");
            let Ok(start) = NaiveDateTime::parse_from_str(start.get(..16).unwrap_or(""), "%Y-%m-%dT%H:%M") else {
                continue;
            };
            if get("dav_state") == "deleted" {
                continue;
            }
            out.push(Event {
                key: p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                start,
                title: get("title"),
                attendees: list_items(front, "attendees"),
                all_day: get("all_day") == "true",
            });
        }
    }
    out
}

/// What was said: events and (event, matter) pairs by when they start, and what the user stopped.
#[derive(Default)]
struct Said {
    events: BTreeMap<String, i64>,
    pairs: BTreeMap<String, i64>,
    stopped: BTreeSet<String>,
}

impl Said {
    fn file(paths: &Paths) -> std::path::PathBuf {
        paths.data.join("ahead.json")
    }

    fn load(paths: &Paths) -> Said {
        let v: Value = std::fs::read_to_string(Self::file(paths))
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null);
        let map = |k: &str| -> BTreeMap<String, i64> {
            v[k].as_object()
                .map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_i64()?))).collect())
                .unwrap_or_default()
        };
        let stopped = v["stopped"]
            .as_array()
            .map(|a| a.iter().filter_map(Value::as_str).filter(|s| stop_key(s)).map(str::to_string).collect())
            .unwrap_or_default();
        Said { events: map("events"), pairs: map("pairs"), stopped }
    }

    /// Forget what started more than a day ago (it cannot come again). -> whether anything was forgotten
    fn prune(&mut self, now: NaiveDateTime) -> bool {
        let before = (now - TimeDelta::days(1)).and_utc().timestamp_millis();
        let n = self.events.len() + self.pairs.len();
        self.events.retain(|_, t| *t >= before);
        self.pairs.retain(|_, t| *t >= before);
        n != self.events.len() + self.pairs.len()
    }

    fn save(&self, paths: &Paths) -> std::io::Result<()> {
        crate::fsutil::ensure_private_dir(&paths.data)?;
        let v = json!({"events": self.events, "pairs": self.pairs, "stopped": self.stopped});
        crate::fsutil::write_private_atomic(&Self::file(paths), v.to_string().as_bytes())
    }
}

/// An ask as it comes over HTTP: `{"events": [{key, start: epoch ms, title, attendees: [..], all_day}], "stopped":
/// [..]}`. None when it is not that shape.
pub fn ask_from(body: &Value) -> Option<Ask> {
    let o = body.as_object()?;
    let text = |v: Option<&Value>, max: usize| -> Option<String> {
        match v {
            None | Some(Value::Null) => Some(String::new()),
            Some(Value::String(s)) if s.chars().count() <= max => Some(s.clone()),
            _ => None,
        }
    };
    let list = |v: Option<&Value>| -> Option<Vec<String>> {
        match v {
            None | Some(Value::Null) => Some(Vec::new()),
            Some(Value::Array(a)) if a.len() <= 200 => {
                a.iter().map(|x| x.as_str().filter(|s| s.len() <= 300).map(str::to_string)).collect()
            }
            _ => None,
        }
    };
    let events = match o.get("events") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(a)) if a.len() <= MAX_EVENTS => a
            .iter()
            .map(|e| {
                let e = e.as_object()?;
                let start = chrono::DateTime::from_timestamp_millis(e.get("start")?.as_i64()?)?
                    .with_timezone(&chrono::Local)
                    .naive_local();
                Some(Event {
                    key: text(e.get("key"), 200)?,
                    start,
                    title: text(e.get("title"), 500)?,
                    attendees: list(e.get("attendees"))?,
                    all_day: match e.get("all_day") {
                        None | Some(Value::Null) => false,
                        Some(v) => v.as_bool()?,
                    },
                })
            })
            .collect::<Option<_>>()?,
        _ => return None,
    };
    Some(Ask { events, stopped: list(o.get("stopped"))? })
}
