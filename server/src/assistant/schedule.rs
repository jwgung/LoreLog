//! The schedule (#62, decided there): the calendar is read, not made. The events the vault holds (CalDAV's `Events/`
//! notes and the phone's calendar, #77, each once as the index has them) from a day to a day, each with the
//! assistant's notes:
//!   - a request someone of the event, or their organization, made and the user has not answered (as #60 judges it:
//!     [`Librarian::open_requests_at`]);
//!   - how the same matter last stood: what the model read is a guess and says so (`추정`, with its words, #64);
//!   - the last record with those people or their organization, its day and title;
//!   - the open to-dos of the same matter.
//!
//! The notes are templates, not the model's words, and only of what is certain (an attendee address that is exactly a
//! person or an organization of the graph); else an event has none. Each event carries the question the chat opens
//! with when it is tapped.
use std::collections::BTreeSet;
use std::time::Duration;

use chrono::{Datelike, NaiveDate};
use serde_json::{Value, json};

use super::ahead::{Unanswered, open_todos, shown};
use crate::config::Paths;
use crate::librarian::Librarian;
use crate::librarian::graph::Structure;
use crate::librarian::index::Listed;

/// the days shown when none are asked for: today and the six after it
pub const DAYS: u64 = 7;
/// the most days one ask may cover
pub const MAX_DAYS: i64 = 31;
/// the requests noted on one event
const REQUESTS: usize = 2;

/// One note on an event.
#[derive(Clone, Debug, PartialEq)]
pub struct Note {
    /// `request`, `state`, `last` or `todo`
    pub kind: &'static str,
    pub text: String,
    /// the model's reading, said as a guess
    pub guess: bool,
}

/// An event with its notes.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub key: String,
    pub day: String,
    /// `HH:MM`, "" for a day's own event
    pub start: String,
    pub end: String,
    pub all_day: bool,
    pub title: String,
    pub place: String,
    /// their names (an address when no name)
    pub attendees: Vec<String>,
    pub notes: Vec<Note>,
    /// what the chat opens with when it is tapped
    pub question: String,
}

impl Item {
    pub fn json(&self) -> Value {
        let notes: Vec<Value> =
            self.notes.iter().map(|n| json!({"kind": n.kind, "text": n.text, "guess": n.guess})).collect();
        json!({"key": self.key, "day": self.day, "start": self.start, "end": self.end, "all_day": self.all_day,
               "title": self.title, "place": self.place, "attendees": self.attendees, "notes": notes,
               "question": self.question})
    }
}

/// The events from `from` to `to` (both included) with their notes, as known on `today` (a past event's state and
/// last record as known on its day; its requests as they stand now). An index not made yet is an error
/// ([`crate::librarian::Outdated`]).
pub fn between(paths: &Paths, from: NaiveDate, to: NaiveDate, today: NaiveDate) -> anyhow::Result<Vec<Item>> {
    let lib = Librarian::open(paths, None, Duration::from_secs(5))?;
    let events = lib.events_between(&ymd(from), &ymd(to))?;
    let todos = super::todos::load(paths);
    let structure = Structure::load(paths);
    let mut out = Vec::new();
    for e in events {
        let notes = notes(&lib, &todos, &structure, &e, today)?;
        out.push(item(e, notes));
    }
    Ok(out)
}

fn notes(lib: &Librarian, todos: &Value, structure: &Structure, e: &Listed, today: NaiveDate) -> anyhow::Result<Notes> {
    let addrs: Vec<String> = e.attendees.iter().map(|(_, a)| a.clone()).filter(|a| a.contains('@')).collect();
    let mut notes = Notes::default();
    // a day off on a team calendar or a company event is not about the requests of some of its people
    if addrs.is_empty() || e.all_day || super::ahead::crowded(&addrs) {
        return Ok(notes);
    }
    // what was by the event's day, or by today for one to come
    let until = ymd(today.min(NaiveDate::parse_from_str(&e.day, "%Y-%m-%d").unwrap_or(today)));
    let opens: Vec<_> = lib.open_requests_at(&addrs)?.into_iter().take(REQUESTS).collect();
    let mut matters: Vec<String> = Vec::new();
    for m in e.case.iter().chain(opens.iter().map(|o| &o.case)) {
        if !matters.contains(m) {
            matters.push(m.clone());
        }
    }
    for o in &opens {
        let said = Unanswered::of(o);
        notes.question.get_or_insert(said.question);
        notes.list.push(Note { kind: "request", text: said.text, guess: false });
    }
    if let Some((st, _)) = matters.iter().find_map(|m| lib.states(m, &until)) {
        let s = |k: &str| st[k].as_str().unwrap_or("").trim().to_string();
        let guess = s("origin") == "model";
        // the stage by the user's name for it, then the record's word (#63)
        let label = match structure.group_of(&s("label")) {
            Some((g, true)) => format!("{g}({})", shown(&s("label"))),
            _ => shown(&s("label")),
        };
        let mut why = vec![words_of(&s("since"), year(&e.day))];
        let ev = super::recall::evidence(&s("evidence"));
        if guess && !ev.is_empty() {
            why.push(format!("근거 \"{ev}\""));
        }
        why.retain(|w| !w.is_empty());
        let mark = if guess { "추정 " } else { "" };
        let why = if why.is_empty() { String::new() } else { format!(" ({})", why.join(", ")) };
        notes.list.push(Note { kind: "state", text: format!("{mark}마지막 상태: {label}{why}"), guess });
    }
    if let Some(r) = lib.last_record_at(&addrs, &until)? {
        let text = format!(
            "{} 마지막 기록: {} {} 「{}」",
            shown(&r.who),
            words_of(&r.day, year(&e.day)),
            r.kind,
            shown(&r.title)
        );
        notes.list.push(Note { kind: "last", text, guess: false });
    }
    let subjects: BTreeSet<String> = matters.iter().flat_map(|m| lib.case_subjects(m)).collect();
    let open = open_todos(todos, &subjects.into_iter().collect::<Vec<_>>());
    if !open.is_empty() {
        notes.list.push(Note { kind: "todo", text: format!("할 일: {}", open.join(", ")), guess: false });
    }
    Ok(notes)
}

#[derive(Default)]
struct Notes {
    list: Vec<Note>,
    question: Option<String>,
}

fn item(e: Listed, notes: Notes) -> Item {
    let question = notes.question.unwrap_or_else(|| {
        format!("{} 「{}」 일정, 관련된 지난 기록과 남은 일 알려 줘", words_of(&e.day, year(&e.day)), shown(&e.title))
    });
    Item {
        key: e.key,
        day: e.day,
        start: e.start,
        end: e.end,
        all_day: e.all_day,
        title: e.title,
        place: e.place,
        attendees: e
            .attendees
            .into_iter()
            .map(|(name, addr)| if name.trim().is_empty() { addr } else { name.trim().to_string() })
            .collect(),
        notes: notes.list,
        question,
    }
}

fn ymd(d: NaiveDate) -> String {
    d.format("%Y-%m-%d").to_string()
}

fn year(day: &str) -> i32 {
    NaiveDate::parse_from_str(day, "%Y-%m-%d").map(|d| d.year()).unwrap_or(0)
}

/// `3월 26일`, with its year when that is not `year`.
fn words_of(day: &str, year: i32) -> String {
    match NaiveDate::parse_from_str(day.get(..10).unwrap_or(day), "%Y-%m-%d") {
        Ok(d) if d.year() == year => format!("{}월 {}일", d.month(), d.day()),
        Ok(d) => format!("{}년 {}월 {}일", d.year(), d.month(), d.day()),
        Err(_) => day.to_string(),
    }
}

/// The days a `GET /schedule` asks for: `from` and `to` (`YYYY-MM-DD`, from not after to, at most [`MAX_DAYS`]),
/// else the week from today. None for anything else.
pub fn days_from(from: Option<&str>, to: Option<&str>, today: NaiveDate) -> Option<(NaiveDate, NaiveDate)> {
    let day = |s: &str| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok();
    let (from, to) = match (from, to) {
        (None, None) => (today, today + chrono::Days::new(DAYS - 1)),
        (Some(f), Some(t)) => (day(f)?, day(t)?),
        _ => return None,
    };
    (from <= to && (to - from).num_days() < MAX_DAYS).then_some((from, to))
}
