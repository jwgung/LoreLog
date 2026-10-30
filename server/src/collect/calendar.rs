//! Import events (VEVENT) from CalDAV calendars into the vault as `Events/` notes. A read-only mirror: nothing is
//! written to the server.
//!
//! - Fetches the events of a window (default 400 days back to 400 days ahead) with REPORT. When the server expands
//!   recurring events, each occurrence becomes its own note.
//! - Notes use the frontmatter of the Calendar Notes plugin's `Events/` notes.
//! - An event is recognised by (dav_uid, start). Only events whose etag changed are rewritten. Notes of events that
//!   disappeared from the server are not deleted.
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;
use std::time::Duration;

use base64::Engine;
use chrono::{Local, NaiveDate, NaiveDateTime, TimeZone};
use regex::Regex;

use crate::assistant::util::files_like;
use crate::errors::kind;

pub const DAV: &str = "DAV:";
pub const CALDAV: &str = "urn:ietf:params:xml:ns:caldav";

/// One event as the note needs it.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Event {
    pub uid: String,
    pub title: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start: String,
    pub end: Option<String>,
    pub all_day: bool,
    pub attendees: Vec<String>,
    pub organizer: Option<String>,
    pub status: Option<String>,
}

fn unfold(text: &str) -> Vec<String> {
    static FOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\r?\n[ \t]").unwrap());
    FOLD.replace_all(text, "").lines().map(str::to_string).collect()
}

fn unescape(v: &str) -> String {
    v.replace("\\n", "\n").replace("\\N", "\n").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

/// `NAME;PARAM=x:value` -> (NAME, params, value)
fn parse_prop(line: &str) -> (String, HashMap<String, String>, String) {
    let (head, value) = line.split_once(':').unwrap_or((line, ""));
    let mut parts = head.split(';');
    let name = parts.next().unwrap_or("").to_uppercase();
    let params = parts
        .map(|x| {
            let (k, v) = x.split_once('=').unwrap_or((x, ""));
            (k.to_uppercase(), v.trim_matches('"').to_string())
        })
        .collect();
    (name, params, value.to_string())
}

/// iCal date/time -> (local naive ISO text, all-day). A TZID that is not known leaves the time as written.
pub fn to_local(value: &str, params: &HashMap<String, String>) -> Option<(String, bool)> {
    let value = value.trim();
    if value.len() == 8 {
        let d = NaiveDate::parse_from_str(value, "%Y%m%d").ok()?;
        return Some((d.format("%Y-%m-%d").to_string(), true));
    }
    let utc = value.ends_with('Z');
    let mut t = NaiveDateTime::parse_from_str(value.trim_end_matches('Z'), "%Y%m%dT%H%M%S").ok()?;
    if utc {
        t = chrono::Utc.from_utc_datetime(&t).with_timezone(&Local).naive_local();
    } else if let Some(tz) = params.get("TZID").and_then(|z| z.parse::<chrono_tz::Tz>().ok())
        && let Some(z) = tz.from_local_datetime(&t).earliest()
    {
        t = z.with_timezone(&Local).naive_local();
    }
    Some((t.format("%Y-%m-%dT%H:%M:%S").to_string(), false))
}

fn addr(value: &str, params: &HashMap<String, String>) -> String {
    static MAILTO: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^mailto:").unwrap());
    let a = MAILTO.replace(value.trim(), "").split_whitespace().collect::<String>();
    let cn = params.get("CN").map(|c| c.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
    if cn.is_empty() { format!("<{a}>") } else { format!("{cn} <{a}>") }
}

/// The events (with a uid and a start) of an iCalendar text.
pub fn parse_events(ics: &str) -> Vec<Event> {
    let mut events = Vec::new();
    let mut cur: Option<Event> = None;
    for line in unfold(ics) {
        let upper = line.to_uppercase();
        if upper == "BEGIN:VEVENT" {
            cur = Some(Event::default());
        } else if upper == "END:VEVENT" {
            if let Some(e) = cur.take() {
                events.push(e);
            }
        } else if let Some(e) = cur.as_mut()
            && line.contains(':')
        {
            let (name, params, value) = parse_prop(&line);
            match name.as_str() {
                "UID" => e.uid = value.trim().to_string(),
                "SUMMARY" => e.title = Some(unescape(&value).trim().to_string()),
                "DESCRIPTION" => e.description = Some(unescape(&value).trim().to_string()),
                "LOCATION" => e.location = Some(unescape(&value).trim().to_string()),
                "DTSTART" => {
                    if let Some((s, all_day)) = to_local(&value, &params) {
                        e.start = s;
                        e.all_day = all_day;
                    }
                }
                "DTEND" => e.end = to_local(&value, &params).map(|x| x.0),
                "ATTENDEE" => e.attendees.push(addr(&value, &params)),
                "ORGANIZER" => e.organizer = Some(addr(&value, &params)),
                "STATUS" => e.status = Some(value.trim().to_string()),
                _ => {}
            }
        }
    }
    events.into_iter().filter(|e| !e.uid.is_empty() && !e.start.is_empty()).collect()
}

/// A title as a file name: no characters Obsidian or the OS cannot take, at most 80 characters.
pub fn safe_name(title: &str) -> String {
    static BAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"[/\\:*?"<>#^\[\]]"#).unwrap());
    static WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    let s = BAD.replace_all(&title.replace('|', "_"), " ").to_string();
    // at most 80 letters and 200 bytes: with the date, a counter and ".md" the name stays under 255 bytes
    let mut s: String = WS.replace_all(&s, " ").trim().chars().take(80).collect();
    while s.len() > 200 {
        s.pop();
    }
    let s = s.trim().to_string();
    if s.is_empty() { "untitled".into() } else { s }
}

/// A YAML scalar: as it is when plain, else double-quoted.
pub fn yaml_str(s: &str) -> String {
    static WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    static SPECIAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"[:#\[\]{}&*!|>'"%@`,]"#).unwrap());
    let s = WS.replace_all(s, " ").trim().to_string();
    if s.is_empty() || SPECIAL.is_match(&s) || s.starts_with(['-', '?']) {
        return format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""));
    }
    s
}

/// One event -> (path in the vault, note text).
pub fn render(ev: &Event, source: &str, etag: &str) -> (String, String) {
    static URL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"https?://").unwrap());
    let title = ev.title.clone().filter(|t| !t.is_empty()).unwrap_or_else(|| "(untitled)".into());
    let location = ev.location.clone().unwrap_or_default();
    let description = ev.description.clone().unwrap_or_default();
    let mut lines = vec![
        "---".to_string(),
        "type: event".into(),
        format!("dav_uid: {}", ev.uid),
        format!("dav_source: {source}"),
        format!("dav_etag: {}", if etag.is_empty() { String::new() } else { yaml_str(etag) }),
        "dav_state: live".into(),
        format!("title: {}", yaml_str(&title)),
        format!("start: {}", ev.start),
        format!("end: {}", ev.end.clone().unwrap_or_else(|| ev.start.clone())),
        format!("all_day: {}", ev.all_day),
        format!("location: {}", yaml_str(&location)),
        format!("online: {}", URL.is_match(&format!("{location}{description}"))),
    ];
    if !ev.attendees.is_empty() {
        // the server sends ATTENDEEs in a different order on every call: sorted, a note never changes without a reason
        lines.push("attendees:".into());
        lines.extend(ev.attendees.iter().collect::<BTreeSet<_>>().into_iter().map(|a| format!("  - {a}")));
    }
    if let Some(o) = &ev.organizer {
        lines.push(format!("organizer: {o}"));
    }
    lines.push("---".into());
    let start_day: String = ev.start.chars().take(10).collect();
    (
        format!("Events/{start_day} {}.md", safe_name(&title)),
        format!("{}\n%% dav:begin %%\n{description}\n", lines.join("\n")),
    )
}

/// Compare etags however they were quoted in the note (`'"x"'`, `"\"x\""` or bare).
pub fn etag_key(v: &str) -> String {
    v.chars().filter(|c| !matches!(c, '\\' | '\'' | '"')).collect::<String>().trim().to_string()
}

/// (dav_uid, start) -> (note path, etag) of the event notes in the vault.
pub fn existing_events(vault: &std::path::Path) -> HashMap<(String, String), (PathBuf, String)> {
    let mut out = HashMap::new();
    for p in files_like(&vault.join("Events"), "", ".md") {
        let head = crate::note::head(&p, 2500);
        let get = |k: &str| crate::note::field(&head, k).map(str::to_string);
        if let (Some(u), Some(s)) = (get("dav_uid").filter(|v| !v.is_empty()), get("start").filter(|v| !v.is_empty())) {
            out.insert((u, s), (p.clone(), get("dav_etag").unwrap_or_default()));
        }
    }
    out
}

// ---------- CalDAV ----------

pub struct Account {
    pub user: String,
    pub password: String,
}

impl Account {
    fn auth(&self) -> String {
        format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", self.user, self.password))
        )
    }
}

/// An HTTP failure: the status when the server answered, else what went wrong (never the URL's credentials).
#[derive(Debug)]
pub enum DavError {
    Status(u16),
    Other(String),
}

impl std::fmt::Display for DavError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DavError::Status(c) => write!(f, "HTTP {c}"),
            DavError::Other(k) => write!(f, "{k}"),
        }
    }
}

/// The account's password goes along in every request: only to https URLs without credentials of their own.
pub fn check_url(url: &str) -> Result<(), String> {
    let u = url::Url::parse(url).map_err(|_| "not a URL".to_string())?;
    if u.scheme() != "https" {
        return Err("a CalDAV URL must use https (the password is sent with every request)".into());
    }
    if !u.username().is_empty() || u.password().is_some() {
        return Err("a CalDAV URL must not carry an account; the account lives in env".into());
    }
    Ok(())
}

fn dav(method: &str, url: &str, acct: &Account, depth: &str, body: String, timeout: u64) -> Result<String, DavError> {
    check_url(url).map_err(DavError::Other)?;
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(timeout)))
        .allow_non_standard_methods(true) // REPORT, PROPFIND
        .build()
        .into();
    let req = ureq::http::Request::builder()
        .method(ureq::http::Method::from_bytes(method.as_bytes()).map_err(|_| DavError::Other("bad method".into()))?)
        .uri(url)
        .header("Depth", depth)
        .header("Content-Type", "application/xml; charset=utf-8")
        .header("Authorization", acct.auth())
        .body(body)
        .map_err(|_| DavError::Other("bad request".into()))?;
    match agent.run(req) {
        Ok(mut r) => {
            r.body_mut().with_config().limit(200 * 1024 * 1024).read_to_string().map_err(|e| DavError::Other(kind(&e)))
        }
        Err(ureq::Error::StatusCode(c)) => Err(DavError::Status(c)),
        Err(e) => Err(DavError::Other(kind(&e))),
    }
}

/// REPORT calendar-query for the window, expanding recurrences when asked.
pub fn report(url: &str, acct: &Account, start: &str, end: &str, expand: bool) -> Result<String, DavError> {
    let exp = if expand { format!("<c:expand start=\"{start}\" end=\"{end}\"/>") } else { String::new() };
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><c:calendar-query xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:prop><d:getetag/><c:calendar-data>{exp}</c:calendar-data></d:prop><c:filter><c:comp-filter name=\"VCALENDAR\"><c:comp-filter name=\"VEVENT\"><c:time-range start=\"{start}\" end=\"{end}\"/></c:comp-filter></c:comp-filter></c:filter></c:calendar-query>"
    );
    dav("REPORT", url, acct, "1", body, 60)
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// calendar-multiget: the event data of specific resources.
fn multiget(url: &str, acct: &Account, hrefs: &[String]) -> Result<String, DavError> {
    let refs: String = hrefs.iter().map(|h| format!("<d:href>{}</d:href>", xml_escape(h))).collect();
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><c:calendar-multiget xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:prop><d:getetag/><c:calendar-data/></d:prop>{refs}</c:calendar-multiget>"
    );
    dav("REPORT", url, acct, "1", body, 120)
}

fn text_of<'a>(node: roxmltree::Node<'a, 'a>, ns: &str, name: &str, deep: bool) -> String {
    let found = if deep {
        node.descendants().find(|n| n.has_tag_name((ns, name)))
    } else {
        node.children().find(|n| n.has_tag_name((ns, name)))
    };
    found
        .map(|n| n.children().filter(|c| c.is_text()).map(|c| c.text().unwrap_or("")).collect::<String>())
        .unwrap_or_default()
}

/// (etag, data, href) of every response in a multistatus.
fn responses(xml: &str) -> Result<Vec<(String, String, String)>, DavError> {
    let doc = roxmltree::Document::parse(xml).map_err(|_| DavError::Other("ParseError".into()))?;
    Ok(doc
        .root_element()
        .children()
        .filter(|n| n.has_tag_name((DAV, "response")))
        .map(|r| {
            (
                text_of(r, DAV, "getetag", true),
                text_of(r, CALDAV, "calendar-data", true),
                text_of(r, DAV, "href", false).trim().to_string(),
            )
        })
        .collect())
}

/// (etag, ics) of the events in a query answer. Some servers (e.g. NAVER WORKS) answer a calendar-query with etags
/// only; the data of those resources is fetched with calendar-multiget in batches. The collection itself is skipped.
pub fn events_from(url: &str, acct: &Account, xml: &str, batch: usize) -> Result<Vec<(String, String)>, DavError> {
    let own = url::Url::parse(url).map(|u| u.path().trim_end_matches('/').to_string()).unwrap_or_default();
    let mut out = Vec::new();
    let mut missing = Vec::new();
    for (etag, data, href) in responses(xml)? {
        if !data.is_empty() {
            out.push((etag, data));
        } else if !href.is_empty() && href.trim_end_matches('/') != own {
            missing.push(href);
        }
    }
    for chunk in missing.chunks(batch) {
        out.extend(
            responses(&multiget(url, acct, chunk)?)?.into_iter().filter(|r| !r.1.is_empty()).map(|(e, d, _)| (e, d)),
        );
    }
    Ok(out)
}

fn propfind(url: &str, acct: &Account, props: &str, depth: &str) -> Result<String, DavError> {
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><d:propfind xmlns:d=\"DAV:\" xmlns:c=\"urn:ietf:params:xml:ns:caldav\"><d:prop>{props}</d:prop></d:propfind>"
    );
    dav("PROPFIND", url, acct, depth, body, 60)
}

/// A URL for the journal: scheme, host and path only (never an account).
fn shown(url: &str) -> String {
    url::Url::parse(url)
        .map(|u| format!("{}://{}{}", u.scheme(), u.host_str().unwrap_or(""), u.path()))
        .unwrap_or_else(|_| "(not a URL)".into())
}

/// `href` (as a CalDAV server returns it) against `at`, if the result is on `base`'s server (scheme, host, port).
/// The account's password goes with every request, so an address the server names elsewhere is never followed.
pub fn same_server(base: &str, at: &str, href: &str) -> Option<String> {
    let origin = |u: &url::Url| u.origin().ascii_serialization();
    let b = url::Url::parse(base).ok()?;
    let joined = url::Url::parse(at).ok()?.join(href).ok()?;
    (origin(&joined) == origin(&b)).then(|| joined.to_string())
}

/// CalDAV discovery: principal -> calendar home -> calendars, all on `base`'s server. -> [(name, url, components)]
pub fn list_calendars(base: &str, acct: &Account) -> Result<Vec<(String, String, String)>, String> {
    check_url(base)?;
    let join = |b: &str, h: &str| same_server(base, b, h);
    let step = |what: &str, url: &str, props: &str, depth: &str| -> Option<String> {
        match propfind(url, acct, props, depth) {
            Ok(x) => Some(x),
            Err(e) => {
                log::warn!("  [{what}] {} -> {e}", shown(url));
                None
            }
        }
    };
    let find = |xml: &str, ns: &str, outer: &str| -> Option<String> {
        let doc = roxmltree::Document::parse(xml).ok()?;
        let o = doc.descendants().find(|n| n.has_tag_name((ns, outer)))?;
        o.descendants().find(|n| n.has_tag_name((DAV, "href"))).and_then(|n| n.text()).map(str::to_string)
    };
    let mut home = None;
    let starts = [Some(base.to_string()), join(base, "/.well-known/caldav"), join(base, "/")];
    for start in starts.into_iter().flatten() {
        let Some(xml) = step("discover", &start, "<d:current-user-principal/><c:calendar-home-set/>", "0") else {
            continue;
        };
        let mut at = start.clone();
        let mut h = find(&xml, CALDAV, "calendar-home-set");
        if h.is_none()
            && let Some(p) = find(&xml, DAV, "current-user-principal").and_then(|p| join(&start, &p))
        {
            at = p;
            h = step("principal", &at, "<c:calendar-home-set/>", "0")
                .and_then(|x| find(&x, CALDAV, "calendar-home-set"));
        }
        if let Some(h) = h.and_then(|h| join(&at, &h)) {
            home = Some(h);
            break;
        }
    }
    let home = home.ok_or("Could not find the calendar home. See the HTTP codes above.")?;
    let xml = step("list", &home, "<d:displayname/><d:resourcetype/><c:supported-calendar-component-set/>", "1")
        .ok_or("listing failed")?;
    let doc = roxmltree::Document::parse(&xml).map_err(|_| "listing is not XML")?;
    let mut out = Vec::new();
    for r in doc.root_element().children().filter(|n| n.has_tag_name((DAV, "response"))) {
        let is_cal = r.descendants().any(|n| {
            n.has_tag_name((DAV, "resourcetype")) && n.children().any(|c| c.has_tag_name((CALDAV, "calendar")))
        });
        if !is_cal {
            continue;
        }
        let comps: Vec<String> = r
            .descendants()
            .filter(|n| n.has_tag_name((CALDAV, "comp")))
            .filter_map(|n| n.attribute("name"))
            .map(str::to_string)
            .collect();
        let name = r
            .descendants()
            .find(|n| n.has_tag_name((DAV, "displayname")))
            .and_then(|n| n.text())
            .unwrap_or("(unnamed)")
            .to_string();
        let Some(url) = join(&home, &text_of(r, DAV, "href", false)) else { continue }; // another server's: not ours
        out.push((name, url, if comps.is_empty() { "?".into() } else { comps.join(",") }));
    }
    Ok(out)
}

// ---------- a sync run ----------

#[derive(Debug, Default, PartialEq)]
pub struct Counts {
    pub created: usize,
    pub updated: usize,
    pub same: usize,
}

/// Write the events of one calendar's answer into the vault. `known` is updated with the notes written.
pub fn apply(
    vault: &std::path::Path,
    source: &str,
    items: &[(String, String)],
    known: &mut HashMap<(String, String), (PathBuf, String)>,
    dry: bool,
    counts: &mut Counts,
) -> std::io::Result<usize> {
    let mut n = 0;
    for (etag, ics) in items {
        for ev in parse_events(ics) {
            n += 1;
            let key = (ev.uid.clone(), ev.start.clone());
            let (rel, note) = render(&ev, source, etag);
            if let Some((path, old)) = known.get(&key) {
                if etag_key(old) == etag_key(etag) {
                    counts.same += 1;
                    continue;
                }
                if !dry {
                    crate::fsutil::write_private_atomic(path, note.as_bytes())?; // 0600, like every record
                }
                counts.updated += 1;
            } else {
                let mut f = vault.join(&rel);
                let mut i = 2;
                while f.exists() {
                    f = vault.join(rel.replace(".md", &format!(" ({i}).md")));
                    i += 1;
                }
                if !dry {
                    fs::create_dir_all(f.parent().unwrap_or(vault))?;
                    crate::fsutil::write_private_atomic(&f, note.as_bytes())?;
                    known.insert(key, (f, etag.clone()));
                }
                counts.created += 1;
            }
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_follows_only_addresses_on_the_same_server() {
        let base = "https://cal.example/dav/";
        assert_eq!(same_server(base, base, "/principals/me/").as_deref(), Some("https://cal.example/principals/me/"));
        assert_eq!(same_server(base, base, "home/").as_deref(), Some("https://cal.example/dav/home/"));
        assert_eq!(same_server(base, base, "https://evil.example/steal/"), None, "another host");
        assert_eq!(same_server(base, base, "http://cal.example/x/"), None, "the same host without TLS");
        assert_eq!(same_server(base, base, "https://cal.example:8443/x/"), None, "another port");
        assert!(list_calendars("http://cal.example/", &Account { user: "u".into(), password: "p".into() }).is_err());
    }

    #[test]
    fn an_event_is_parsed_and_rendered_like_the_calendar_plugin_notes() {
        let ics = "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:u1@x\r\nSUMMARY:이현민 | 회의\\, 킥오프\r\nDTSTART;TZID=Asia/Seoul:20260921T130000\r\nDTEND;TZID=Asia/Seoul:20260921T140000\r\nLOCATION:소회의실 3\r\nATTENDEE;CN=\"김한얼\";PARTSTAT=NEEDS-ACTION:mailto:hekim@x.com\r\nORGANIZER;CN=송진호:mailto:jinhsong@x.com\r\nDESCRIPTION:첫 줄\\n둘째\r\n  줄\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let evs = parse_events(ics);
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].title.as_deref(), Some("이현민 | 회의, 킥오프"));
        assert_eq!(evs[0].description.as_deref(), Some("첫 줄\n둘째 줄"), "folded lines are joined");
        let seoul: chrono_tz::Tz = "Asia/Seoul".parse().unwrap();
        let want = seoul.with_ymd_and_hms(2026, 9, 21, 13, 0, 0).unwrap().with_timezone(&Local).naive_local();
        assert_eq!(evs[0].start, want.format("%Y-%m-%dT%H:%M:%S").to_string());
        let (rel, note) = render(&evs[0], "caldav/test", "\"abc\"");
        assert_eq!(rel, format!("Events/{} 이현민 _ 회의, 킥오프.md", &evs[0].start[..10]));
        assert!(
            note.contains("dav_uid: u1@x\n")
                && note.contains("  - 김한얼 <hekim@x.com>\n")
                && note.contains("title: \"이현민 | 회의, 킥오프\"\n")
        );
        assert!(note.contains("dav_etag: \"\\\"abc\\\"\"\n"));
        assert!(note.ends_with("---\n%% dav:begin %%\n첫 줄\n둘째 줄\n"));
        assert_eq!(to_local("20260921", &HashMap::new()), Some(("2026-09-21".into(), true)));
    }

    #[test]
    fn etags_compare_however_they_were_quoted() {
        let k = etag_key("\"2025-10-29 15:38:34\"");
        assert_eq!(etag_key("\"\\\"2025-10-29 15:38:34\\\"\""), k);
        assert_eq!(etag_key("'\"2025-10-29 15:38:34\"'"), k);
        assert_eq!(k, "2025-10-29 15:38:34");
    }

    #[test]
    fn a_query_answer_with_data_is_used_as_it_is_and_the_collection_is_never_fetched() {
        let xml = r#"<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav"><d:response><d:href>/cal/</d:href><d:propstat><d:prop><d:getetag>"c"</d:getetag></d:prop></d:propstat></d:response><d:response><d:href>/cal/a.ics</d:href><d:propstat><d:prop><d:getetag>"e1"</d:getetag><c:calendar-data>BEGIN:VCALENDAR</c:calendar-data></d:prop></d:propstat></d:response></d:multistatus>"#;
        let acct = Account { user: "u".into(), password: "p".into() };
        assert_eq!(
            events_from("https://h/cal/", &acct, xml, 50).unwrap(),
            [("\"e1\"".to_string(), "BEGIN:VCALENDAR".to_string())]
        );
    }
}
