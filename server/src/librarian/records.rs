//! Turn the records the user keeps into documents for the librarian, under the same rules the diary uses.
//!
//! The raw records stay as they are; this only decides what the index may hold:
//!   mail, events      every one, body included (the user's own mailbox and calendar)
//!   phone (lorelog)   messenger notifications only from senders the user allowed; voice only inside conversations the
//!                     diary counts as the user's (the same rule the diary's context uses); calls/texts; app use.
//!                     Screen text is not indexed (the diary does not read it), nor location (only coordinates).
//!   PC                window titles
//!   the user's words  what the user said in chat, reflections and corrections. The assistant's replies and its daily
//!                     summary are derived from records, not records, so they are not indexed.
//!
//! A document is {source, kind, day, title, text} plus, when there is one, `extra` (searched but not shown: a mail's
//! sender, recipients and attachment names, an event's attendees) and `thread` (the first message id of a mail's
//! References header, else In-Reply-To, else its own id; replies of one thread come back as one hit, mails that only
//! share a subject do not). Traces (phone, PC) become one document per day and kind; mail, events and words are one
//! document each.
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::assistant::context::{Conversations, confirmed_voice_label};
use crate::assistant::diary::split_correction;
use crate::assistant::senders::{noise, record_allowed};
use crate::assistant::util::{Counter, as_int, files_like, files_like_nested, hm, local, pystr, truthy};
use crate::config::Paths;
use crate::note;

type Record = Map<String, Value>;

static DAY_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\d{4}-\d{2}-\d{2}").unwrap());
static MSG_ID_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"[^\s<>,\[\]"']+@[^\s<>,\[\]"']+"#).unwrap());
/// frontmatter lines that only carry sync bookkeeping, not anything a person would search for
static NOISE_KEYS: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?m)^(type|mail_id|mail_state|mail_synced|references|in_reply_to|richtung|dav_uid|dav_source|dav_etag|dav_state):.*\n?")
        .unwrap()
});
static OBSIDIAN_COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)%%.*?%%").unwrap());
static NOTE_HEADING: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?m)^#{1,6} (Notes|Message)[ \t]*$").unwrap());
static BLANKS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n{3,}").unwrap());

/// A conversation longer than this is not believed: the index then decides again about every phone file.
pub const MAX_CONVERSATION_DAYS: i64 = 31;

#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub source: &'static str,
    pub kind: String,
    pub day: String,
    pub title: String,
    pub text: String,
    pub extra: String,
    pub thread: Option<String>,
    /// who the record is with: mail headers, event attendees (#22)
    pub people: Vec<Person>,
}

/// Someone a record names: their role in it, the name it gives them (may be empty) and their address (lower case).
#[derive(Clone, Debug, PartialEq)]
pub struct Person {
    /// "from", "to", "cc" (mail) or "attendee" (event)
    pub role: &'static str,
    pub name: String,
    pub addr: String,
}

/// At most this many people are kept of one header (a mail to more is a mass mail either way).
const MAX_PEOPLE: usize = 200;

/// The `name <addr>` (or bare `addr`) entries of a header value or list.
pub fn addresses(value: &str, role: &'static str) -> Vec<Person> {
    static ENTRY: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?:([^<>,\[\]]*?)\s*<([^<>\s,]+@[^<>\s,]+)>|([^\s<>,\[\]"']+@[^\s<>,\[\]"']+))"#).unwrap()
    });
    ENTRY
        .captures_iter(value)
        .filter_map(|c| {
            let addr =
                c.get(2).or_else(|| c.get(3))?.as_str().trim_matches(|ch| ch == '"' || ch == '\'').to_lowercase();
            let name =
                c.get(1).map(|m| m.as_str().trim().trim_matches(|ch| ch == '"' || ch == '\'').trim()).unwrap_or("");
            Some(Person { role, name: name.chars().take(100).collect(), addr })
        })
        .take(MAX_PEOPLE)
        .collect()
}

/// The lines of a YAML list under `key:` (`  - item`), up to the next key.
fn list_items(front: &str, key: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in front.lines() {
        if inside {
            match line.trim_start().strip_prefix("- ") {
                Some(item) if line.starts_with(' ') || line.starts_with('-') => out.push(item.trim().to_string()),
                _ if line.trim().is_empty() => {}
                _ => break,
            }
        } else if line.strip_prefix(key).is_some_and(|r| r.starts_with(':')) {
            inside = true;
            let rest = line[key.len() + 1..].trim();
            if !rest.is_empty() {
                out.push(rest.to_string()); // an inline value: `key: [a, b]`
            }
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    Mail,
    Event,
    Lorelog,
    Pc,
    Diary,
    /// the phone's address book, sent as whole lists (`snap`): no documents, only who is who (#22)
    Contacts,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Mail => "mail",
            Kind::Event => "event",
            Kind::Lorelog => "lorelog",
            Kind::Pc => "pc",
            Kind::Diary => "diary",
            Kind::Contacts => "contacts",
        }
    }
}

/// Every file the librarian reads, in a stable order (by path).
pub fn sources(paths: &Paths) -> Vec<(Kind, PathBuf)> {
    let mut out: Vec<(Kind, PathBuf)> =
        files_like_nested(&paths.vault.join("Mail"), "", ".md").into_iter().map(|p| (Kind::Mail, p)).collect();
    out.extend(files_like(&paths.vault.join("Events"), "", ".md").into_iter().map(|p| (Kind::Event, p)));
    for (kind, name) in
        [(Kind::Lorelog, "lorelog"), (Kind::Pc, "pc"), (Kind::Diary, "diary"), (Kind::Contacts, "contacts")]
    {
        out.extend(files_like(&paths.raw().join(name), "", ".ndjson").into_iter().map(|p| (kind, p)));
    }
    out.sort_by(|a, b| a.1.cmp(&b.1));
    out
}

/// Fingerprint of the rules that decide what phone records may be indexed (the allowed senders).
pub fn policy_stamp(allow: &BTreeSet<String>) -> String {
    hex::encode(Sha256::digest(serde_json::to_string(allow).expect("strings serialize").as_bytes()))
}

/// The days a conversation spans (the voice it can contain lies on these days).
#[derive(Debug, PartialEq)]
pub enum Span {
    /// no usable span: it holds nothing
    Nothing,
    Days(Vec<String>),
    /// its days cannot be listed (longer than [`MAX_CONVERSATION_DAYS`], or times that are numbers but no date):
    /// the diary's voice rule compares the numbers, so it can still hold voice on any day
    Unlistable,
}

pub fn conversation_days(c: &Record) -> Span {
    let (Some(start), Some(end)) = (as_int(c.get("start_ms")), as_int(c.get("end_ms"))) else { return Span::Nothing };
    let (Some(first), Some(last)) = (local(start).map(|t| t.date()), local(end).map(|t| t.date())) else {
        return Span::Unlistable;
    };
    if last < first {
        return Span::Days(vec![first.format("%Y-%m-%d").to_string()]);
    }
    let n = (last - first).num_days();
    if n >= MAX_CONVERSATION_DAYS {
        return Span::Unlistable;
    }
    Span::Days((0..=n).map(|i| (first + chrono::Duration::days(i)).format("%Y-%m-%d").to_string()).collect())
}

// ---------- notes ----------

/// The value of the first `key:` line, trimmed and without surrounding quotes.
fn front_value(front: &str, key: &str) -> String {
    note::field(front, key).unwrap_or_default().trim_matches('"').trim_matches('\'').to_string()
}

/// The `YYYY-MM-DD` a value starts with, else the one the file name starts with, else "".
fn day_of(value: &str, fallback_name: &str) -> String {
    DAY_RE.find(value).or_else(|| DAY_RE.find(fallback_name)).map(|m| m.as_str().to_string()).unwrap_or_default()
}

/// The mail body as a person reads it: without Obsidian comments (%% … %%) and the note's section headings.
pub fn clean_body(body: &str) -> String {
    let b = OBSIDIAN_COMMENT.replace_all(body, "");
    let b = NOTE_HEADING.replace_all(&b, "");
    BLANKS.replace_all(&b, "\n\n").trim().to_string()
}

/// The thread a mail belongs to: the first id in References, else In-Reply-To, else the mail's own id.
fn thread_of(front: &str, path: &Path) -> String {
    for key in ["references", "in_reply_to", "mail_id"] {
        if let Some(m) = MSG_ID_RE.find(&front_value(front, key)) {
            return format!("mail:{}", m.as_str().to_lowercase());
        }
    }
    format!("mail:{}", file_name(path))
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn mail_docs(path: &Path) -> std::io::Result<Vec<Doc>> {
    let text = note::read(path)?;
    let (front, body) = note::split(&text);
    let sent = front_value(front, "mail_source").contains("Sent");
    let subject = front_value(front, "subject");
    Ok(vec![Doc {
        source: "mail",
        kind: if sent { "보낸 메일" } else { "받은 메일" }.into(),
        day: day_of(&front_value(front, "date"), &file_name(path)),
        title: if subject.is_empty() { "(제목 없음)".into() } else { subject },
        thread: Some(thread_of(front, path)),
        extra: NOISE_KEYS.replace_all(front, "").trim().to_string(), // who, to whom, attachments: searched, not shown
        text: clean_body(body),
        people: ["from", "to", "cc"]
            .into_iter()
            .flat_map(|role| addresses(note::field(front, role).unwrap_or_default(), role))
            .collect(),
    }])
}

pub fn event_docs(path: &Path) -> std::io::Result<Vec<Doc>> {
    let text = note::read(path)?;
    let (front, body) = note::split(&text);
    let (start, end, place) = (front_value(front, "start"), front_value(front, "end"), front_value(front, "location"));
    let head: String = start.chars().take(16).collect::<String>().replace('T', " ");
    let tail: String = end.chars().skip(11).take(5).collect();
    let when = format!("{head}~{tail}").trim_matches('~').to_string();
    let title = front_value(front, "title");
    let place_line = if place.is_empty() { String::new() } else { format!("장소 {place}") };
    let parts: Vec<String> = [when, place_line, clean_body(body)].into_iter().filter(|x| !x.is_empty()).collect();
    Ok(vec![Doc {
        source: "event",
        kind: "일정".into(),
        day: day_of(&start, &file_name(path)),
        title: if title.is_empty() { stem(path) } else { title },
        extra: NOISE_KEYS.replace_all(front, "").trim().to_string(),
        thread: None,
        text: parts.join("\n"),
        people: list_items(front, "attendees").iter().flat_map(|a| addresses(a, "attendee")).collect(),
    }])
}

// ---------- traces ----------

fn trace_docs(day: &str, groups: Vec<(&str, Vec<String>)>) -> Vec<Doc> {
    groups
        .into_iter()
        .filter(|(_, lines)| !lines.is_empty())
        .map(|(kind, lines)| Doc {
            source: "trace",
            kind: kind.into(),
            day: day.into(),
            title: format!("{day} {kind}"),
            text: lines.join("\n"),
            extra: String::new(),
            thread: None,
            people: Vec::new(),
        })
        .collect()
}

/// The documents of one day's phone records, given the conversation states and the allowed senders. A line whose
/// values have the wrong shape (a list for a name, a time too large to be one) is left out alone.
pub fn lorelog_docs(day: &str, lines: &[Record], convs: &Conversations, allow: &BTreeSet<String>) -> Vec<Doc> {
    let (mut msgs, mut comm, mut voice) = (Vec::new(), Vec::new(), Vec::new());
    let mut apps: Counter<String> = Counter::default();
    for d in lines {
        let t = d.get("type").and_then(Value::as_str).unwrap_or("");
        let at = hm(as_int(d.get("ts")));
        match t {
            "notification" if !noise(d) && record_allowed(allow, d) => {
                let text = if d.contains_key("text") { pystr(d.get("text")) } else { String::new() };
                let line = format!("{at} {}: {} — {text}", pystr(d.get("label")), pystr(d.get("title")));
                msgs.push(line.trim_end_matches([' ', '—']).to_string());
            }
            "comm" => {
                let party = [d.get("party_name"), d.get("party")]
                    .into_iter()
                    .find(|v| truthy(*v))
                    .map(|v| pystr(v))
                    .unwrap_or_default();
                let content = if d.contains_key("content") { pystr(d.get("content")) } else { String::new() };
                comm.push(
                    format!("{at} {}/{} {party} {content}", pystr(d.get("kind")), pystr(d.get("direction")))
                        .trim()
                        .to_string(),
                );
            }
            "voice" if truthy(d.get("text")) => {
                let Some(ts) = as_int(d.get("ts")) else { continue };
                if let Some(who) = confirmed_voice_label(convs.values(), ts) {
                    voice.push(format!("{at} (with {who}) {}", pystr(d.get("text"))));
                }
            }
            "app" if !noise(d) && truthy(d.get("label")) => {
                if matches!(d.get("label"), Some(Value::Array(_) | Value::Object(_))) {
                    continue; // a name that is a list or a map is no name
                }
                apps.add(pystr(d.get("label")));
            }
            _ => {}
        }
    }
    let app_line: Vec<String> = if apps.is_empty() {
        Vec::new()
    } else {
        vec![apps.most_common(usize::MAX).into_iter().map(|(k, v)| format!("{k} x{v}")).collect::<Vec<_>>().join(", ")]
    };
    trace_docs(day, vec![("메신저", msgs), ("통화/문자", comm), ("대화", voice), ("앱", app_line)])
}

pub fn pc_docs(path: &Path) -> std::io::Result<Vec<Doc>> {
    let mut seen = std::collections::HashSet::new();
    let mut lines = Vec::new();
    for d in crate::assistant::context::read_records(path, None)? {
        if d.get("type").and_then(Value::as_str) == Some("window") && truthy(d.get("app")) {
            let key = (pystr(d.get("app")), d.get("title").map(|v| pystr(Some(v))).unwrap_or_default());
            if seen.insert(key.clone()) {
                lines.push(format!("{} {} {}", hm(as_int(d.get("ts"))), key.0, key.1).trim().to_string());
            }
        }
    }
    Ok(trace_docs(&stem(path), vec![("PC", lines)]))
}

// ---------- the user's words ----------

fn word(kind: &str, day: &str, text: String) -> Doc {
    Doc {
        source: "word",
        kind: kind.into(),
        day: day.into(),
        title: kind.into(),
        text,
        extra: String::new(),
        thread: None,
        people: Vec::new(),
    }
}

/// The text of the document a line the user said in the chat becomes (a correction reads "wrong → right").
pub fn word_text(said: &str) -> String {
    match split_correction(said) {
        Some((wrong, right)) => format!("{wrong} → {right}"),
        None => said.to_string(),
    }
}

/// A record that cannot be understood: its file is skipped (and tried again on the next update).
#[derive(Debug)]
pub struct NotUnderstood(pub String);

impl std::fmt::Display for NotUnderstood {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for NotUnderstood {}

/// What the user said: chat lines, reflections, corrections. A chat line is also saved as a reflection record; the same
/// words are indexed once.
pub fn word_docs(path: &Path) -> anyhow::Result<Vec<Doc>> {
    let records = crate::assistant::context::read_records(path, None)?;
    let file_day = stem(path);
    let day_of_rec = |d: &Record| {
        day_of(
            &match d.get("date") {
                None | Some(Value::Null) => String::new(),
                v => pystr(v),
            },
            &file_day,
        )
    };
    let mut out = Vec::new();
    let mut said = std::collections::HashSet::new();
    for d in &records {
        let text = d.get("text").and_then(Value::as_str).unwrap_or("");
        if d.get("type").and_then(Value::as_str) == Some("chat")
            && d.get("role").and_then(Value::as_str) == Some("user")
            && !text.is_empty()
        {
            let day = day_of_rec(d);
            match split_correction(text) {
                Some((_, right)) => {
                    said.insert(right.trim().to_string());
                    out.push(word("정정", &day, word_text(text)));
                }
                None => {
                    said.insert(text.trim().to_string());
                    out.push(word("대화에서 한 말", &day, text.to_string()));
                }
            }
        }
    }
    for d in &records {
        if d.get("type").and_then(Value::as_str) != Some("reflection") {
            continue;
        }
        let day = day_of_rec(d);
        let text =
            if truthy(d.get("reflection")) { pystr(d.get("reflection")).trim().to_string() } else { String::new() };
        if !text.is_empty() && !said.contains(&text) {
            out.push(word("회고", &day, text));
        }
        let corrections: Vec<Value> = match d.get("corrections") {
            None | Some(Value::Null) | Some(Value::Bool(false)) => Vec::new(),
            Some(Value::Array(a)) => a.clone(),
            Some(Value::String(_)) | Some(Value::Object(_)) => Vec::new(), // their parts are not corrections
            Some(other) => return Err(NotUnderstood(format!("corrections is {other}")).into()),
        };
        for c in corrections.iter().filter_map(Value::as_object) {
            if truthy(c.get("wrong")) && truthy(c.get("right")) && !said.contains(pystr(c.get("right")).trim()) {
                out.push(word("정정", &day, format!("{} → {}", pystr(c.get("wrong")), pystr(c.get("right")))));
            }
        }
    }
    Ok(out)
}

/// The documents of a mail, event, PC or words file (phone files go through [`lorelog_docs`]).
pub fn docs(kind: Kind, path: &Path) -> anyhow::Result<Vec<Doc>> {
    Ok(match kind {
        Kind::Mail => mail_docs(path)?,
        Kind::Event => event_docs(path)?,
        Kind::Pc => pc_docs(path)?,
        Kind::Diary | Kind::Lorelog => word_docs(path)?,
        Kind::Contacts => Vec::new(), // read as a whole by the index (only the newest list counts)
    })
}

/// One entry of an address book: the names and the addresses it puts together (mail in lower case, phone numbers as
/// `tel:<digits>`, a leading 82 read as 0).
#[derive(Clone, Debug, PartialEq)]
pub struct Contact {
    pub name: String,
    pub addrs: Vec<String>,
}

/// A phone number as compared: its digits, `+82 10-1234-5678` and `010 1234 5678` alike.
pub fn phone_key(number: &str) -> Option<String> {
    let digits: String = number.chars().filter(char::is_ascii_digit).collect();
    let digits = match digits.strip_prefix("82") {
        Some(rest) if number.trim_start().starts_with('+') || digits.len() >= 11 => format!("0{rest}"),
        _ => digits,
    };
    (digits.len() >= 7).then(|| format!("tel:{digits}"))
}

/// The newest address book in these files: the lines of the highest `snap` (a list sent whole; an older list, or a
/// contact deleted since, does not count).
pub fn newest_contacts(files: &[PathBuf]) -> Vec<Contact> {
    let mut lines: Vec<Value> = Vec::new();
    for f in files {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        lines.extend(
            text.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()).filter(|v| v["type"] == "contact"),
        );
    }
    let Some(snap) = lines.iter().filter_map(|v| as_int(v.get("snap"))).max() else { return Vec::new() };
    let list = |v: &Value, k: &str| -> Vec<String> {
        v.get(k)
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default()
    };
    lines
        .iter()
        .filter(|v| as_int(v.get("snap")) == Some(snap))
        .map(|v| {
            let mut addrs: Vec<String> =
                list(v, "emails").iter().map(|e| e.trim().to_lowercase()).filter(|e| e.contains('@')).collect();
            addrs.extend(list(v, "phones").iter().filter_map(|p| phone_key(p)));
            addrs.dedup();
            Contact { name: pystr(v.get("name")).trim().chars().take(100).collect(), addrs }
        })
        .filter(|c| !c.addrs.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_numbers_compare_by_their_digits() {
        assert_eq!(phone_key("+82 10-1234-5678").as_deref(), Some("tel:01012345678"));
        assert_eq!(phone_key("010 1234 5678").as_deref(), Some("tel:01012345678"));
        assert_eq!(phone_key("02-123-4567").as_deref(), Some("tel:021234567"));
        assert_eq!(phone_key("1234"), None, "too short to be someone's number");
    }

    #[test]
    fn mail_headers_and_attendees_give_names_and_addresses() {
        let p = addresses(r#"["김영희 수석" <YH@x.com>, bare@y.com, 이름만]"#, "to");
        let got: Vec<(&str, &str)> = p.iter().map(|p| (p.name.as_str(), p.addr.as_str())).collect();
        assert_eq!(got, [("김영희 수석", "yh@x.com"), ("", "bare@y.com")]);
        let front = "title: 회의\nattendees:\n  - 영희 <b@x.com>\n  - c@x.com\nlocation: 2층\n";
        assert_eq!(list_items(front, "attendees"), ["영희 <b@x.com>", "c@x.com"]);
    }
    use serde_json::json;

    #[test]
    fn a_conversations_days_are_listed_up_to_a_month() {
        let ms = |d: u32| {
            chrono::TimeZone::with_ymd_and_hms(&chrono::Local, 2024, 4, d, 9, 0, 0).unwrap().timestamp_millis()
        };
        let c = |s: i64, e: Value| json!({"start_ms": s, "end_ms": e}).as_object().unwrap().clone();
        assert_eq!(
            conversation_days(&c(ms(10), json!(ms(13)))),
            Span::Days(vec!["2024-04-10".into(), "2024-04-11".into(), "2024-04-12".into(), "2024-04-13".into()])
        );
        assert_eq!(conversation_days(&c(ms(10), json!(10_i64.pow(17)))), Span::Unlistable);
        assert_eq!(conversation_days(&c(ms(1), json!(ms(1) + 40 * 86_400_000))), Span::Unlistable);
        assert_eq!(conversation_days(&json!({"start_ms": 1}).as_object().unwrap().clone()), Span::Nothing);
    }

    #[test]
    fn a_mail_body_loses_obsidian_comments_and_note_headings() {
        assert_eq!(clean_body("## Notes\n\n%% mail-sync:begin %%\n## Message\n\n\n\n본문\n"), "본문");
    }

    #[test]
    fn a_thread_is_the_first_reference() {
        let front = "references: [<Root@x>, <r1@x>]\nin_reply_to: r1@x\nmail_id: me@x\n";
        assert_eq!(thread_of(front, Path::new("a.md")), "mail:root@x");
        assert_eq!(thread_of("mail_id: \n", Path::new("a.md")), "mail:a.md");
    }
}
