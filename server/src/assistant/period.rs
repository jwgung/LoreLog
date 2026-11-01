//! What hangs in a period (#61): "이번 달에 뭐가 걸려 있지". The model only names the period ([`Period`]); its days are
//! counted here from the day asked, and what hangs in them is gathered in code, from four places:
//!   1. to-dos open or proposed, their free-text due read by [`due`] (one it cannot read is "기한 불명", at the end);
//!   2. calendar events in the days (with no event ahead and no calendar server, the section says it cannot see ahead);
//!   3. requests to the user not answered, of the last 30 days (the graph);
//!   4. matters with records in the last 30 days whose last state is not an end (the graph).
//!
//! Overdue first, then by day; a few of each ([`CAPS`]) and at most [`MAX_LINES`] lines, the rest counted. Calendar
//! events and to-dos the user took are facts; proposals, requests and states are what the model read, marked `추정`.
use std::sync::LazyLock;

use chrono::{Datelike, Duration, NaiveDate, Weekday};
use regex::Regex;

use super::recall::{MAX_EVIDENCE, Recalled, Source, plain};

/// The periods the model can name. Their days are counted from the day asked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Period {
    ThisWeek,
    NextWeek,
    ThisMonth,
    NextMonth,
    /// "요즘": the next two weeks
    Recent,
}

/// The days of a period, both included, and whether what is past its due counts (a period that starts today).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    pub from: NaiveDate,
    pub to: NaiveDate,
    pub overdue: bool,
}

/// Requests and matters count when they had a record this many days back at most.
pub const RECENT_DAYS: i64 = 30;
/// "요즘": today and the days after it
pub const RECENT_AHEAD: i64 = 13;

impl Period {
    /// The name the model gives (`this_month`, …).
    pub fn from_name(name: &str) -> Option<Period> {
        Some(match name.trim().to_lowercase().as_str() {
            "this_week" => Period::ThisWeek,
            "next_week" => Period::NextWeek,
            "this_month" => Period::ThisMonth,
            "next_month" => Period::NextMonth,
            "recent" => Period::Recent,
            _ => return None,
        })
    }

    pub fn label(self) -> &'static str {
        match self {
            Period::ThisWeek => "이번 주",
            Period::NextWeek => "다음 주",
            Period::ThisMonth => "이번 달",
            Period::NextMonth => "다음 달",
            Period::Recent => "요즘(앞으로 2주)",
        }
    }

    /// Its days from `today`. A period that starts today ("이번 달" = today to the month's end, #61) also holds what
    /// is overdue; one that starts later does not.
    pub fn span(self, today: NaiveDate) -> Span {
        let monday = week_start(today);
        let (from, to, overdue) = match self {
            Period::ThisWeek => (today, monday + Duration::days(6), true),
            Period::NextWeek => (monday + Duration::days(7), monday + Duration::days(13), false),
            Period::ThisMonth => (today, month_end(today), true),
            Period::NextMonth => {
                let first = month_end(today) + Duration::days(1);
                (first, month_end(first), false)
            }
            Period::Recent => (today, today + Duration::days(RECENT_AHEAD), true),
        };
        Span { from, to, overdue }
    }
}

fn week_start(d: NaiveDate) -> NaiveDate {
    d - Duration::days(d.weekday().num_days_from_monday() as i64)
}

fn month_end(d: NaiveDate) -> NaiveDate {
    let (y, m) = if d.month() == 12 { (d.year() + 1, 1) } else { (d.year(), d.month() + 1) };
    NaiveDate::from_ymd_opt(y, m, 1).expect("first of a month") - Duration::days(1)
}

fn add_months(d: NaiveDate, n: u32) -> NaiveDate {
    let m0 = d.month0() + n;
    NaiveDate::from_ymd_opt(d.year() + (m0 / 12) as i32, m0 % 12 + 1, 1).expect("first of a month")
}

// ---------- the due a to-do was written with ----------

static FULL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(\d{4})\s*[-./년]\s*(\d{1,2})\s*[-./월]\s*(\d{1,2})").unwrap());
static MONTH_DAY: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?:^|[^\d])(\d{1,2})\s*(?:/|\.|월\s*)(\d{1,2})(?:[^\d]|$)").unwrap());
static MONTH_PART: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{1,2})\s*월\s*(말|중순|초)").unwrap());
static AFTER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\d{1,2})\s*(일|주)\s*(?:후|뒤|내|이내|안)").unwrap());
static DAY_ONLY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?:^|[^\d])(\d{1,2})\s*일").unwrap());
static WEEK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(다다음\s*주|다음\s*주|담주|차주|이번\s*주|금주)").unwrap());
static WEEKDAY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([월화수목금토일])\s*요일").unwrap());
/// a weekday written short, right after a week (`담주 화`)
static SHORT_WEEKDAY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*([월화수목금토일])(?:\s|$|까지)").unwrap());

fn weekday(c: &str) -> Option<Weekday> {
    Some(match c {
        "월" => Weekday::Mon,
        "화" => Weekday::Tue,
        "수" => Weekday::Wed,
        "목" => Weekday::Thu,
        "금" => Weekday::Fri,
        "토" => Weekday::Sat,
        "일" => Weekday::Sun,
        _ => return None,
    })
}

/// The day a to-do's due (as the model or the user wrote it, `금요일까지`, `11/5`, `다음 주 화요일`, `월말`) falls
/// on, counted from the day the to-do was written. None when the words name no day.
pub fn due(text: &str, written: NaiveDate) -> Option<NaiveDate> {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let num = |s: &str| s.parse::<u32>().ok();
    if let Some(c) = FULL.captures(&t) {
        return NaiveDate::from_ymd_opt(c[1].parse().ok()?, num(&c[2])?, num(&c[3])?);
    }
    // a month of no year: this year's, unless that is long past (`1/10` written in November is next January)
    let in_year = |m: u32, d: u32| -> Option<NaiveDate> {
        let day = NaiveDate::from_ymd_opt(written.year(), m, d)?;
        if day < written - Duration::days(90) { NaiveDate::from_ymd_opt(written.year() + 1, m, d) } else { Some(day) }
    };
    if let Some(c) = MONTH_PART.captures(&t) {
        let first = in_year(num(&c[1])?, 1)?;
        return Some(part_of_month(first, &c[2]));
    }
    if let Some(c) = MONTH_DAY.captures(&t) {
        return in_year(num(&c[1])?, num(&c[2])?);
    }
    if let Some(c) = AFTER.captures(&t) {
        let n = num(&c[1])? as i64;
        return Some(written + Duration::days(if &c[2] == "주" { n * 7 } else { n }));
    }
    if let Some(w) = WEEK.captures(&t) {
        let word = w[1].replace(' ', "");
        let offset = match word.as_str() {
            "다다음주" => 2,
            "다음주" | "담주" | "차주" => 1,
            _ => 0,
        };
        let monday = week_start(written) + Duration::days(7 * offset);
        let rest = &t[w.get(0).expect("whole match").end()..];
        let wd = WEEKDAY
            .captures(rest)
            .or_else(|| SHORT_WEEKDAY.captures(rest))
            .and_then(|c| weekday(&c[1]))
            .map(|d| d.num_days_from_monday() as i64);
        let days = match wd {
            Some(d) => d,
            None if rest.contains("주말") || t.contains("주말") => 6,
            None => 4, // a week as a due: its working days, to Friday
        };
        return Some(monday + Duration::days(days));
    }
    if t.contains("주말") {
        return Some(week_start(written) + Duration::days(6));
    }
    if let Some(c) = WEEKDAY.captures(&t) {
        let target = weekday(&c[1])?.num_days_from_monday() as i64;
        let today = written.weekday().num_days_from_monday() as i64;
        return Some(written + Duration::days((target - today).rem_euclid(7)));
    }
    let next_month = ["다음 달", "다음달", "담달", "익월", "내달"];
    let this_month = ["이번 달", "이번달", "이달", "금월"];
    if let Some(at) = next_month.iter().find_map(|w| t.find(w).map(|i| i + w.len())) {
        return Some(part_of_month(add_months(written, 1), &t[at..]));
    }
    if t.contains("월말") || t.contains("말일") {
        return Some(month_end(written));
    }
    if let Some(at) = this_month.iter().find_map(|w| t.find(w).map(|i| i + w.len())) {
        return Some(part_of_month(add_months(written, 0), &t[at..]));
    }
    if t.contains("연말") || t.contains("올해") || t.contains("年末") {
        return NaiveDate::from_ymd_opt(written.year(), 12, 31);
    }
    if let Some(c) = DAY_ONLY.captures(&t) {
        let d = num(&c[1])?;
        let this = NaiveDate::from_ymd_opt(written.year(), written.month(), d);
        return match this {
            Some(day) if day >= written => Some(day),
            _ => {
                let next = add_months(written, 1);
                NaiveDate::from_ymd_opt(next.year(), next.month(), d)
            }
        };
    }
    for (word, days) in [("오늘", 0), ("내일", 1), ("모레", 2), ("글피", 3)] {
        if t.contains(word) {
            return Some(written + Duration::days(days));
        }
    }
    None
}

/// A day of the month that starts at `first` by what the words after it say: its start (`초`, the 10th), its middle
/// (`중순`, the 20th), else its end.
fn part_of_month(first: NaiveDate, words: &str) -> NaiveDate {
    let at = |d: u32| NaiveDate::from_ymd_opt(first.year(), first.month(), d).expect("a day every month has");
    if words.contains("초") {
        at(10)
    } else if words.contains("중순") {
        at(20)
    } else {
        month_end(first)
    }
}

// ---------- what the librarian gathers ----------

/// A calendar event in the days.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Event {
    pub day: String,
    /// `HH:MM`, empty for a day's event
    pub time: String,
    pub title: String,
    pub refers: String,
    /// the matter it is in (#8), if any
    pub case: Option<String>,
}

/// A to-do open or proposed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Todo {
    pub title: String,
    /// as written
    pub due: String,
    /// the day it was written
    pub day: String,
    /// found by the model in the records and not taken by the user yet
    pub proposed: bool,
}

/// A request to the user with no mail of theirs after it (what the graph read).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Request {
    pub day: String,
    pub title: String,
    /// what was asked for
    pub what: String,
    pub evidence: String,
    pub refers: String,
    pub case: String,
}

/// A matter with recent records whose last state is not an end.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Matter {
    /// its last record
    pub day: String,
    pub title: String,
    pub state: String,
    pub since: String,
    pub evidence: String,
    /// the state was read by the model (a guess), not taken from the headers
    pub guess: bool,
    pub refers: String,
    pub case: String,
}

/// What the librarian found for a period's days.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Gathered {
    pub events: Vec<Event>,
    /// there are events after today, or a calendar server to bring them: else the days ahead cannot be seen
    pub ahead: bool,
    pub todos: Vec<Todo>,
    /// None: there is no graph to ask (not "none found")
    pub requests: Option<Vec<Request>>,
    pub matters: Option<Vec<Matter>>,
}

/// at most this many lines of each: events, to-dos, requests, matters under way
pub const CAPS: [usize; 4] = [4, 4, 3, 3];
pub const MAX_LINES: usize = 12;

pub const NOT_FOUND: &str = "- (찾지 못함: 이 기간에 걸린 일정·할 일·요청을 찾지 못했다)";
pub const NO_AHEAD: &str = "- (앞 일정은 볼 수 없음: 캘린더 서버가 연결되지 않았고, 폰은 일정이 시작된 뒤에야 보낸다. 앞으로 일정이 없다는 뜻이 아니다)";
pub const NO_GRAPH: &str =
    "- (답하지 않은 요청과 진행 중인 건은 찾아보지 못함: 기록을 읽은 그래프가 아직 없다. 없다는 뜻이 아니다)";

/// Which of the four a line is (the index of its cap).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Branch {
    Event = 0,
    Todo = 1,
    Request = 2,
    Matter = 3,
}

impl Branch {
    fn label(self) -> &'static str {
        ["일정", "할 일", "요청", "진행 중인 건"][self as usize]
    }
}

struct Line {
    branch: Branch,
    /// where it goes: 0 overdue, 1 by day, 2 requests, 3 matters, 4 no due known
    group: u8,
    key: String,
    text: String,
    source: Option<Source>,
}

fn one(s: &str) -> String {
    plain(s).split_whitespace().collect::<Vec<_>>().join(" ")
}

fn short(s: &str) -> String {
    let o = one(s).replace('"', "'");
    if o.chars().count() > MAX_EVIDENCE {
        format!("{}…", o.chars().take(MAX_EVIDENCE).collect::<String>())
    } else {
        o
    }
}

/// Does a line say any of these names (compared without spaces and case)?
fn names_any(text: &str, names: &[String]) -> bool {
    let key = |s: &str| s.split_whitespace().collect::<String>().to_lowercase();
    let t = key(text);
    names.iter().map(|n| key(n)).filter(|n| !n.is_empty()).any(|n| t.contains(&n))
}

/// The section for what hangs in `span` (asked on `today`), narrowed to the lines that say one of `names` when there
/// are any, and the records it quotes.
pub fn render(g: &Gathered, period: Period, span: Span, today: NaiveDate, names: &[String]) -> Recalled {
    let day = |d: NaiveDate| d.format("%Y-%m-%d").to_string();
    let (from, to) = (day(span.from), day(span.to));
    let mut lines: Vec<Line> = Vec::new();
    let src = |d: &str, kind: &str, title: &str, refers: &str| Source {
        day: d.into(),
        kind: kind.into(),
        title: one(title).chars().take(80).collect(),
        refers: refers.into(),
    };
    for e in g.events.iter().filter(|e| e.day >= from && e.day <= to) {
        let when = if e.time.is_empty() { e.day.clone() } else { format!("{} {}", e.day, e.time) };
        lines.push(Line {
            branch: Branch::Event,
            group: 1,
            key: when.clone(),
            text: format!("- {when} [일정] {}", one(&e.title)),
            source: Some(src(&e.day, "일정", &e.title, &e.refers)),
        });
    }
    for t in &g.todos {
        let written = NaiveDate::parse_from_str(&t.day, "%Y-%m-%d").unwrap_or(today);
        let kind = if t.proposed { "[추정 할 일]" } else { "[할 일]" };
        let mut why = Vec::new();
        if t.proposed {
            why.push("비서가 기록에서 뽑았고 사용자가 아직 받지 않음".to_string());
        }
        if !t.due.trim().is_empty() {
            why.push(format!("기한 \"{}\"", short(&t.due)));
        }
        why.push(format!("{}에 적음", t.day));
        let why = why.join(", ");
        let title = one(&t.title);
        match due(&t.due, written) {
            Some(d) if d < span.from && span.overdue && d < today => lines.push(Line {
                branch: Branch::Todo,
                group: 0,
                key: day(d),
                text: format!("- {} {kind} {title} · 기한 넘김 ({why})", day(d)),
                source: None,
            }),
            Some(d) if d >= span.from && d <= span.to => lines.push(Line {
                branch: Branch::Todo,
                group: 1,
                key: day(d),
                text: format!("- {} {kind} {title} ({why})", day(d)),
                source: None,
            }),
            Some(_) => {}
            None => lines.push(Line {
                branch: Branch::Todo,
                group: 4,
                key: t.day.clone(),
                text: format!("- 기한 불명 {kind} {title} ({why})"),
                source: None,
            }),
        }
    }
    // one matter, one line: an event before a request before a state
    let mut cases: Vec<String> = g.events.iter().filter_map(|e| e.case.clone()).collect();
    let mut requests: Vec<&Request> = g.requests.iter().flatten().collect();
    requests.sort_by(|a, b| a.day.cmp(&b.day)); // the longest waiting first
    for r in requests {
        if cases.contains(&r.case) {
            continue;
        }
        cases.push(r.case.clone());
        let what = if r.what.is_empty() { String::new() } else { format!(" · 요청: {}", one(&r.what)) };
        let ev = short(&r.evidence);
        let ev = if ev.is_empty() { String::new() } else { format!(", 근거 \"{ev}\"") };
        lines.push(Line {
            branch: Branch::Request,
            group: 2,
            key: r.day.clone(),
            text: format!(
                "- {} [추정 답하지 않은 요청] {}{what} ({}에 요청, 그 뒤 보낸 메일 없음{ev})",
                r.day,
                one(&r.title),
                r.day
            ),
            source: Some(src(&r.day, "답하지 않은 요청", &r.title, &r.refers)),
        });
    }
    let mut matters: Vec<&Matter> = g.matters.iter().flatten().collect();
    matters.sort_by(|a, b| b.day.cmp(&a.day)); // the latest first
    for m in matters {
        if cases.contains(&m.case) {
            continue;
        }
        cases.push(m.case.clone());
        let ev = short(&m.evidence);
        let mut why = vec![m.since.clone()];
        if m.guess && !ev.is_empty() {
            why.push(format!("근거 \"{ev}\""));
        }
        why.retain(|w| !w.is_empty());
        let mark = if m.guess { "추정 " } else { "" };
        lines.push(Line {
            branch: Branch::Matter,
            group: 3,
            key: m.day.clone(),
            text: format!(
                "- {} [{mark}진행 중인 건] {} · {mark}마지막 상태: {} ({}), 마지막 기록 {}",
                m.day,
                one(&m.title),
                one(&m.state),
                why.join(", "),
                m.day
            ),
            source: Some(src(&m.day, "진행 중인 건", &m.title, &m.refers)),
        });
    }
    if !names.is_empty() {
        lines.retain(|l| names_any(&l.text, names));
    }
    // overdue first, then by day; the requests, the matters, and what has no day known at the end
    lines.sort_by(|a, b| {
        a.group.cmp(&b.group).then_with(|| if a.group <= 1 { a.key.cmp(&b.key) } else { std::cmp::Ordering::Equal })
    });
    let mut taken = [0usize; 4];
    let mut kept = Vec::new();
    let mut left = [0usize; 4];
    for l in lines {
        let b = l.branch as usize;
        if taken[b] < CAPS[b] && kept.len() < MAX_LINES {
            taken[b] += 1;
            kept.push(l);
        } else {
            left[b] += 1;
        }
    }
    let overdue = if span.overdue { ", 기한 넘긴 것 포함" } else { "" };
    let mut out = vec![format!(
        "- (기간: {} {from}~{to}{overdue}. 일정과 사용자가 받은 할 일은 기록 그대로이고, \"추정\"은 기록을 읽고 짐작한 것이다)",
        period.label()
    )];
    if kept.is_empty() {
        out.push(NOT_FOUND.to_string());
    }
    let mut sources = Vec::new();
    for l in kept {
        out.push(l.text);
        sources.extend(l.source);
    }
    let more: usize = left.iter().sum();
    if more > 0 {
        let by: Vec<String> = [Branch::Event, Branch::Todo, Branch::Request, Branch::Matter]
            .iter()
            .filter(|b| left[**b as usize] > 0)
            .map(|b| format!("{} {}", b.label(), left[*b as usize]))
            .collect();
        out.push(format!("- (외 {more}건: {})", by.join(", ")));
    }
    if !g.ahead && span.to > today {
        out.push(NO_AHEAD.to_string());
    }
    if g.requests.is_none() || g.matters.is_none() {
        out.push(NO_GRAPH.to_string());
    }
    Recalled { section: out.join("\n"), sources }
}
