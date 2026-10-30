//! Everything the assistant reads about a day: mail, events, calls, allowed messenger notifications, confirmed voice,
//! PC windows. The result is the `[오늘 기록]` part of every prompt, in the same words the Python server wrote.
use std::collections::{BTreeSet, HashSet};
use std::fs;
use std::path::Path;
use std::sync::LazyLock;

use chrono::{Duration, Local, NaiveDate, NaiveDateTime};
use indexmap::IndexMap;
use regex::Regex;
use serde_json::{Map, Value};

use super::senders::{load_allow, noise, record_allowed};
use super::util::{Counter, as_int, files_like, files_like_nested, front, local, pystr, take, truthy};
use crate::config::Paths;

type Record = Map<String, Value>;

static PREFIX_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^(\s*(re|fw|fwd|회신|전달)\s*:\s*)+").unwrap());
static KEEP_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^0-9a-z가-힣]").unwrap());
static BULK_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(no-?reply|newsletter|news@|info@|support@|marketing|promo|stibee|mailchimp|광고|뉴스레터|\(광고\))",
    )
    .unwrap()
});
static MAIL_NAME_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(\d{4}-\d{2}-\d{2})-(\d{2})(\d{2})-").unwrap());

// ---------- records ----------

/// The JSON objects of an NDJSON file, in order. Undecodable bytes and lines that are not objects are passed over
/// (raw files are only ever appended to, so one bad line must not hide the rest). With `only`, lines that do not
/// contain that text are not decoded at all.
pub fn read_records(path: &Path, only: Option<&str>) -> std::io::Result<Vec<Record>> {
    let bytes = fs::read(path)?;
    Ok(String::from_utf8_lossy(&bytes)
        .lines()
        .filter(|l| only.map(|o| l.contains(o)).unwrap_or(true))
        .filter_map(|l| match serde_json::from_str::<Value>(l) {
            Ok(Value::Object(o)) => Some(o),
            _ => None,
        })
        .collect())
}

/// Records of a file, or none when it is missing or unreadable.
fn records_or_none(path: &Path) -> Vec<Record> {
    read_records(path, None).unwrap_or_default()
}

/// The conversation records among a phone file's records, in file order. One without an id is passed over.
pub fn conversations_in(lines: Vec<Record>) -> Vec<Record> {
    lines
        .into_iter()
        .filter(|d| d.get("type").and_then(Value::as_str) == Some("conversation") && truthy_id(d))
        .collect()
}

fn truthy_id(d: &Record) -> bool {
    !matches!(d.get("id"), None | Some(Value::Null))
}

/// The key a conversation is known by (its id as JSON, so 1 and "1" stay apart as they did in Python).
pub fn conversation_key(c: &Record) -> String {
    c.get("id").map(|v| v.to_string()).unwrap_or_default()
}

/// Conversations by id, latest state wins, first-seen order kept.
pub type Conversations = IndexMap<String, Record>;

/// Latest state of every phone conversation (an updated status arrives as a new line, maybe in another day's file).
/// A file that cannot be read right now is passed over.
pub fn load_conversations(paths: &Paths) -> Conversations {
    let mut convs = Conversations::new();
    for path in files_like(&paths.raw().join("lorelog"), "", ".ndjson") {
        match read_records(&path, None) {
            Ok(lines) => {
                for c in conversations_in(lines) {
                    convs.insert(conversation_key(&c), c);
                }
            }
            Err(e) => eprintln!(
                "conversations: skipped {}: {:?}",
                path.file_name().unwrap_or_default().to_string_lossy(),
                e.kind()
            ),
        }
    }
    convs
}

/// Participants of the conversation this voice belongs to if the user confirmed it as theirs, else None.
/// ambient / pending / skipped conversations and voice outside any conversation are never used. A conversation
/// without a usable time span cannot contain anything.
pub fn confirmed_voice_label<'a>(convs: impl IntoIterator<Item = &'a Record>, ts_ms: i64) -> Option<String> {
    for c in convs {
        let status = c.get("status").and_then(Value::as_str);
        if !matches!(status, Some("tagged") | Some("auto")) {
            continue;
        }
        let (Some(start), Some(end)) = (as_int(c.get("start_ms")), as_int(c.get("end_ms"))) else {
            continue;
        };
        if start <= ts_ms && ts_ms <= end {
            return Some(if truthy(c.get("participants")) { pystr(c.get("participants")) } else { "?".into() });
        }
    }
    None
}

// ---------- mail and events ----------

/// Subject without RE:/FW: prefixes, reduced to letters and digits, for matching a reply to its request.
pub fn subj_key(subject: &str) -> String {
    let sub = PREFIX_RE.replace(subject, "");
    KEEP_RE.replace_all(&sub.to_lowercase(), "").into_owned()
}

/// A mail the user sent: (subject key, in-reply-to without <>, when, epoch seconds).
pub type Sent = (String, String, f64);

/// Mail the user sent in the last `days` days. Only the frontmatter is read.
pub fn sent_index(paths: &Paths, days: i64) -> Vec<Sent> {
    let cutoff = Local::now().date_naive() - Duration::days(days);
    let mut out = Vec::new();
    for p in files_like_nested(&paths.vault.join("Mail"), "", ".md") {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let Some(m) = MAIL_NAME_RE.captures(name) else {
            continue;
        };
        let Ok(day) = NaiveDate::parse_from_str(&m[1], "%Y-%m-%d") else {
            continue;
        };
        if day < cutoff {
            continue;
        }
        let f = front(&p, &["mail_source", "subject", "in_reply_to"]);
        if !f[0].contains("Sent") {
            continue;
        }
        let Some(when) = local_epoch(&format!("{}T{}:{}:00", &m[1], &m[2], &m[3])) else {
            continue;
        };
        out.push((subj_key(&f[1]), f[2].trim_matches(['<', '>']).to_string(), when));
    }
    out
}

/// Epoch seconds of a local `YYYY-MM-DDTHH:MM:SS`.
fn local_epoch(s: &str) -> Option<f64> {
    let t = NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok()?;
    t.and_local_timezone(Local).earliest().map(|t| t.timestamp() as f64)
}

/// Did the user send a reply to this mail after it arrived (same thread subject, or an in-reply-to pointing at it)?
pub fn replied(sent: &[Sent], subject: &str, mail_id: &str, received_at: f64) -> bool {
    let key = subj_key(subject);
    let mid = mail_id.trim_matches(['<', '>']);
    sent.iter().any(|(k, r, w)| *w >= received_at && ((!key.is_empty() && *k == key) || (!mid.is_empty() && r == mid)))
}

/// Mail of the day. Marked, not removed: bulk senders, mail the user sent, mail the user already replied to.
/// 'RE: RE:' chains collapse to one row: the first by file name, so the earliest of the day (the Python server took
/// whichever the file system listed first).
pub fn gather_mail(paths: &Paths, day: &str) -> Vec<String> {
    let sent = sent_index(paths, 21);
    let mut rows = Vec::new();
    let mut seen = HashSet::new();
    for p in files_like_nested(&paths.vault.join("Mail"), &format!("{day}-"), ".md") {
        let f = front(&p, &["time", "from", "subject", "mail_source", "mail_id"]);
        let (time, from, subject, source, mail_id) = (&f[0], &f[1], &f[2], &f[3], &f[4]);
        let subj = PREFIX_RE.replace(subject, "").trim().to_string();
        if !seen.insert((from.clone(), subj.clone())) {
            continue;
        }
        let mark = if source.contains("Sent") {
            " [내가 보냄]"
        } else if BULK_RE.is_match(&format!("{from} {subject}")) {
            " [대량 발송]"
        } else {
            let t = if time.is_empty() { "00:00" } else { time.as_str() };
            let mut hm = t.split(':');
            let (h, m) = (
                hm.next().and_then(|x| x.trim().parse::<u32>().ok()),
                hm.next().and_then(|x| x.trim().parse::<u32>().ok()),
            );
            match h.zip(m).and_then(|(h, m)| local_epoch(&format!("{day}T{h:02}:{m:02}:00"))) {
                Some(received) if replied(&sent, subject, mail_id, received) => " [이미 회신함]",
                _ => "",
            }
        };
        rows.push(format!("{time} {from} — {subj}{mark}"));
    }
    rows.sort();
    rows
}

/// Events of the day: `HH:MM~HH:MM title` (`종일` for an all-day start).
pub fn gather_events(paths: &Paths, day: &str) -> Vec<String> {
    let mut rows: Vec<String> = files_like(&paths.vault.join("Events"), day, ".md")
        .iter()
        .map(|p| {
            let f = front(p, &["title", "start", "end", "all_day"]);
            let start = slice(&f[1], 11, 16);
            format!("{}~{} {}", if start.is_empty() { "종일".to_string() } else { start }, slice(&f[2], 11, 16), f[0])
        })
        .collect();
    rows.sort();
    rows
}

/// Characters `from..to` (like a Python slice, empty when the text is shorter).
fn slice(s: &str, from: usize, to: usize) -> String {
    s.chars().skip(from).take(to.saturating_sub(from)).collect()
}

// ---------- phone ----------

/// What a day's phone file gives the prompt.
#[derive(Default)]
pub struct Lorelog {
    pub voice: Vec<String>,
    pub comm: Vec<String>,
    pub notif: Counter<String>,
    pub msgs: IndexMap<String, Vec<String>>,
    pub apps: Counter<String>,
    pub places: Vec<String>,
}

fn hm_of(t: NaiveDateTime) -> String {
    t.format("%H:%M").to_string()
}

pub fn gather_lorelog(paths: &Paths, day: &str) -> Lorelog {
    let mut out = Lorelog::default();
    let path = paths.raw().join("lorelog").join(format!("{day}.ndjson"));
    let Ok(bytes) = fs::read(&path) else {
        return out;
    };
    let convs = load_conversations(paths);
    let allow: BTreeSet<String> = load_allow(paths);
    let mut places: Vec<(NaiveDateTime, i64, i64)> = Vec::new();
    for line in String::from_utf8_lossy(&bytes).lines() {
        let Ok(Value::Object(d)) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let t = d.get("type").and_then(Value::as_str).unwrap_or("");
        let when = as_int(d.get("ts")).and_then(local);
        match t {
            "voice" if truthy(d.get("text")) => {
                let (Some(ts), Some(at)) = (as_int(d.get("ts")), when) else {
                    continue;
                };
                if let Some(who) = confirmed_voice_label(convs.values(), ts) {
                    out.voice.push(format!("{} (with {who}) {}", hm_of(at), take(&pystr(d.get("text")), 200)));
                }
            }
            "comm" => {
                let Some(at) = when else { continue };
                let party =
                    if truthy(d.get("party_name")) { pystr(d.get("party_name")) } else { pystr(d.get("party")) };
                let content = if d.contains_key("content") { pystr(d.get("content")) } else { String::new() };
                out.comm.push(format!(
                    "{} {}/{} {party} {content}",
                    hm_of(at),
                    pystr(d.get("kind")),
                    pystr(d.get("direction"))
                ));
            }
            "notification" if !noise(&d) && record_allowed(&allow, &d) => {
                let key = format!("{}: {}", pystr(d.get("label")), pystr(d.get("title")));
                out.notif.add(key.clone());
                if truthy(d.get("text")) {
                    let Some(at) = when else { continue };
                    out.msgs.entry(key).or_default().push(format!("{} {}", hm_of(at), take(&pystr(d.get("text")), 80)));
                }
            }
            "app" if !noise(&d) => out.apps.add(pystr(d.get("label"))),
            "location" => {
                let (Some(at), Some(la), Some(lo)) = (when, num(d.get("lat")), num(d.get("lon"))) else {
                    continue;
                };
                places.push((at, (la * 1000.0).round() as i64, (lo * 1000.0).round() as i64));
            }
            _ => {}
        }
    }
    // consecutive points in the same ~100 m cell are one stay
    places.sort();
    let mut stays: Vec<(NaiveDateTime, NaiveDateTime, i64, i64)> = Vec::new();
    for (ts, la, lo) in places {
        match stays.last_mut() {
            Some(s) if (s.2, s.3) == (la, lo) => s.1 = ts,
            _ => stays.push((ts, ts, la, lo)),
        }
    }
    if let (Some(first), Some(last)) = (stays.first(), stays.last()) {
        let cells: HashSet<(i64, i64)> = stays.iter().map(|s| (s.2, s.3)).collect();
        out.places = vec![format!(
            "위치 기록 {}~{}, 서로 다른 장소 {}곳 (이름 없음)",
            hm_of(first.0),
            hm_of(last.1),
            cells.len()
        )];
    }
    out
}

fn num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

// ---------- PC ----------

/// What the user worked in on the PC: the longest stays (at least 5 minutes), the same app and title merged across
/// breaks under 5 minutes, in time order.
pub fn gather_pc(paths: &Paths, day: &str, top: usize) -> Vec<String> {
    struct Stay {
        app: String,
        title: String,
        ts: i64,
        end: i64,
        secs: i64,
    }
    let mut segs: Vec<(i64, i64, String, String, i64)> =
        records_or_none(&paths.raw().join("pc").join(format!("{day}.ndjson")))
            .into_iter()
            .filter(|d| d.get("type").and_then(Value::as_str) == Some("window") && truthy(d.get("app")))
            .filter_map(|d| {
                let (ts, end) = (as_int(d.get("ts")).filter(|v| *v != 0)?, as_int(d.get("end")).filter(|v| *v != 0)?);
                Some((
                    ts,
                    end,
                    pystr(d.get("app")),
                    d.get("title").map(|v| pystr(Some(v))).unwrap_or_default(),
                    as_int(d.get("dur_s")).unwrap_or(0),
                ))
            })
            .collect();
    segs.sort_by_key(|s| s.0);
    let mut merged: Vec<Stay> = Vec::new();
    for (ts, end, app, title, dur) in segs {
        match merged.last_mut() {
            Some(m) if m.app == app && m.title == title && ts - m.end <= 5 * 60 * 1000 => {
                m.end = end;
                m.secs += dur;
            }
            _ => merged.push(Stay { app, title, ts, end, secs: dur }),
        }
    }
    let mut longest: Vec<&Stay> = merged.iter().filter(|m| m.secs >= 300).collect();
    longest.sort_by_key(|m| -m.secs);
    longest.truncate(top);
    longest.sort_by_key(|m| m.ts);
    longest
        .iter()
        .map(|m| {
            let (a, b) = (local(m.ts).map(hm_of).unwrap_or_default(), local(m.end).map(hm_of).unwrap_or_default());
            format!("{a}~{b} {} \"{}\" ({}분)", m.app, take(&m.title, 80), m.secs.div_euclid(60))
        })
        .collect()
}

// ---------- the prompt part ----------

/// The day's records as prompt text. For the summary (`summary`), app use counts and places are left out: they filled
/// the summary with empty sentences ("다양한 앱을 활용하셨어요").
pub fn build_context(paths: &Paths, day: &str, summary: bool) -> String {
    let lore = gather_lorelog(paths, day);
    let mail: Vec<String> = gather_mail(paths, day).into_iter().take(60).collect();
    let notif: Vec<String> = lore
        .notif
        .most_common(15)
        .into_iter()
        .map(|(k, v)| {
            let msgs = lore.msgs.get(&k).map(|m| m[m.len().saturating_sub(4)..].join(" / ")).unwrap_or_default();
            format!("{k} x{v}: {msgs}")
        })
        .collect();
    let mut parts: Vec<(&str, Vec<String>)> = vec![
        ("일정", gather_events(paths, day)),
        ("받거나 보낸 메일(제목만)", mail),
        ("통화/문자", lore.comm.clone()),
        ("대화(사용자가 본인 대화로 확인한 음성 전사, 오류 있음)", lore.voice.iter().take(30).cloned().collect()),
        ("PC에서 5분 넘게 머문 창(앱, 창 제목, 시간)", gather_pc(paths, day, 12)),
        ("메신저 알림(사용자가 허용한 상대만, 건수: 최근 내용)", notif),
        ("앱 사용(기록 횟수)", lore.apps.most_common(12).into_iter().map(|(k, v)| format!("{k} x{v}")).collect()),
        ("위치", lore.places.iter().take(20).cloned().collect()),
    ];
    if summary {
        parts.retain(|(n, _)| *n != "앱 사용(기록 횟수)" && *n != "위치");
    }
    parts
        .iter()
        .map(|(name, rows)| {
            let body = if rows.is_empty() {
                "- (없음)".to_string()
            } else {
                rows.iter().map(|r| format!("- {r}")).collect::<Vec<_>>().join("\n")
            };
            format!("## {name}\n{body}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
