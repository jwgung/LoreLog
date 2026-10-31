//! Address books over CardDAV (#22), the way calendars come in: read-only, https only, the account's password never
//! sent to another server. Each book is kept in the raw store (`raw/contacts/`) as a whole list, only when it changed;
//! the librarian uses the newest list of each book (and of the phone) to know who is who. Everything a card holds is
//! kept, in the same shape the phone sends (and the card itself as it came), but photos and sounds.
use std::collections::BTreeMap;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::calendar::{Account, DavError, check_url, dav, same_server, shown};
use crate::config::Paths;
use crate::store::RawStore;

const DAV: &str = "DAV:";
const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";

/// A contact as kept (the same shape the phone sends): every field the card holds, and the card itself.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Card {
    pub name: String,
    pub nickname: String,
    /// `{type, value}`; mail in lower case
    pub emails: Vec<Value>,
    pub phones: Vec<Value>,
    /// `{company, department, title}`
    pub orgs: Vec<Value>,
    /// `{type, value}`, the parts joined with ", "
    pub addresses: Vec<Value>,
    pub urls: Vec<String>,
    pub notes: Vec<String>,
    /// `{type, date}`: birthday, anniversary
    pub events: Vec<Value>,
    /// `{type, name}`
    pub relations: Vec<Value>,
    /// `{type, value}`
    pub ims: Vec<Value>,
    pub groups: Vec<String>,
    /// the vCard as it came
    pub raw: String,
}

/// vCard text unescaped (`\,` `\;` `\n`).
fn unescape(v: &str) -> String {
    v.replace("\\n", "\n").replace("\\N", "\n").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

/// The components of a structured value (split on `;` not escaped), each unescaped.
fn parts(v: &str) -> Vec<String> {
    let mut out = vec![String::new()];
    let mut esc = false;
    for ch in v.chars() {
        match (esc, ch) {
            (false, '\\') => {
                esc = true;
                out.last_mut().unwrap().push(ch);
            }
            (false, ';') => out.push(String::new()),
            _ => {
                esc = false;
                out.last_mut().unwrap().push(ch);
            }
        }
    }
    out.iter().map(|p| unescape(p).trim().to_string()).collect()
}

/// The TYPE of a property (`EMAIL;TYPE=WORK,PREF` -> "work"), lower case; "" when none.
fn kind_of(head: &str) -> String {
    head.split(';')
        .skip(1)
        .find_map(|p| {
            let (k, v) = p.split_once('=').unwrap_or(("TYPE", p));
            k.eq_ignore_ascii_case("TYPE")
                .then(|| v.split(',').find(|t| !t.eq_ignore_ascii_case("pref")).unwrap_or("").to_lowercase())
        })
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

const MAX_EACH: usize = 50;

/// The cards of a text holding one or more vCards (3.0 or 4.0), with everything they hold but photos and sounds.
pub fn parse_vcards(text: &str) -> Vec<Card> {
    // unfold: a line starting with a space or tab continues the one before
    let unfolded = text.replace("\r\n", "\n").replace("\n ", "").replace("\n\t", "");
    let mut out = Vec::new();
    let mut cur: Option<(String, Card)> = None; // (the name from N, the card)
    for line in unfolded.lines() {
        let Some((head, value)) = line.split_once(':') else {
            if let Some((_, c)) = cur.as_mut() {
                c.raw.push_str(line);
                c.raw.push('\n');
            }
            continue;
        };
        let prop = head.split(';').next().unwrap_or("").rsplit('.').next().unwrap_or("").to_ascii_uppercase();
        if prop == "BEGIN" && value.trim().eq_ignore_ascii_case("VCARD") {
            cur = Some((String::new(), Card::default()));
        }
        let Some((nname, c)) = cur.as_mut() else { continue };
        if !matches!(prop.as_str(), "PHOTO" | "LOGO" | "SOUND" | "KEY") {
            c.raw.push_str(line);
            c.raw.push('\n');
        }
        let value = value.trim();
        let typed = |v: String| json!({"type": kind_of(head), "value": v});
        let push = |list: &mut Vec<Value>, v: Value| {
            if list.len() < MAX_EACH && !list.contains(&v) {
                list.push(v);
            }
        };
        match prop.as_str() {
            "END" if value.eq_ignore_ascii_case("VCARD") => {
                let (nname, mut c) = cur.take().unwrap_or_default();
                if c.name.trim().is_empty() {
                    c.name = nname;
                }
                c.name = c.name.trim().chars().take(200).collect();
                out.push(c);
            }
            "FN" => c.name = unescape(value),
            "N" => {
                let p = parts(value);
                let (family, given) = (p.first().cloned().unwrap_or_default(), p.get(1).cloned().unwrap_or_default());
                let hangul = family.chars().chain(given.chars()).any(|ch| ('가'..='힣').contains(&ch));
                *nname =
                    if hangul { format!("{family}{given}") } else { format!("{given} {family}") }.trim().to_string();
            }
            "NICKNAME" => c.nickname = unescape(value),
            "EMAIL" => {
                let e = value.trim_start_matches("mailto:").to_lowercase();
                if e.contains('@') && e.len() <= 254 {
                    push(&mut c.emails, typed(e));
                }
            }
            "TEL" => {
                let t = value.trim_start_matches("tel:").to_string();
                if !t.is_empty() && t.len() <= 40 {
                    push(&mut c.phones, typed(t));
                }
            }
            "ORG" => {
                let p = parts(value);
                let org = json!({"company": p.first().cloned().unwrap_or_default(), "department": p.get(1).cloned().unwrap_or_default(), "title": ""});
                push(&mut c.orgs, org);
            }
            "TITLE" | "ROLE" => {
                let t = unescape(value);
                match c.orgs.last_mut() {
                    Some(o) if o["title"] == "" => o["title"] = json!(t),
                    _ => push(&mut c.orgs, json!({"company": "", "department": "", "title": t})),
                }
            }
            "ADR" => {
                let joined: Vec<String> = parts(value).into_iter().filter(|p| !p.is_empty()).collect();
                if !joined.is_empty() {
                    push(&mut c.addresses, typed(joined.join(", ")));
                }
            }
            "URL" if c.urls.len() < MAX_EACH => c.urls.push(unescape(value)),
            "NOTE" if c.notes.len() < MAX_EACH => c.notes.push(unescape(value)),
            "BDAY" => push(&mut c.events, json!({"type": "birthday", "date": value})),
            "ANNIVERSARY" | "X-ANNIVERSARY" => push(&mut c.events, json!({"type": "anniversary", "date": value})),
            "RELATED" | "X-ABRELATEDNAMES" => {
                push(&mut c.relations, json!({"type": kind_of(head), "name": unescape(value)}))
            }
            "IMPP" | "X-AIM" | "X-JABBER" | "X-SKYPE" | "X-MSN" | "X-QQ" => push(&mut c.ims, typed(unescape(value))),
            "CATEGORIES" => {
                for g in value.split(',').map(unescape).map(|g| g.trim().to_string()).filter(|g| !g.is_empty()) {
                    if c.groups.len() < MAX_EACH && !c.groups.contains(&g) {
                        c.groups.push(g);
                    }
                }
            }
            _ => {}
        }
    }
    out
}

/// A contact as a raw store line (`snap`, `origin`: whose list it is).
fn line(c: &Card, origin: &str, snap: i64) -> Value {
    json!({"type": "contact", "snap": snap, "ts": snap, "origin": origin, "name": c.name, "nickname": c.nickname,
           "emails": c.emails, "phones": c.phones, "orgs": c.orgs, "addresses": c.addresses, "urls": c.urls,
           "notes": c.notes, "events": c.events, "relations": c.relations, "ims": c.ims, "groups": c.groups,
           "raw": c.raw})
}

/// The raw store lines of a book: one per card, all with the same `snap` and `origin` (whose list it is).
pub fn lines(cards: &[Card], origin: &str, snap: i64) -> Vec<String> {
    if cards.is_empty() {
        // an empty book is kept too (one marker line), so its deleted cards leave the index
        return vec![line(&Card::default(), origin, snap).to_string()];
    }
    cards.iter().map(|c| line(c, origin, snap).to_string()).collect()
}

/// The `origin` of a book: `carddav:` and the start of its URL's hash (the URL itself is not in the records).
pub fn origin(url: &str) -> String {
    format!("carddav:{}", &hex::encode(Sha256::digest(url.as_bytes()))[..12])
}

fn state_file(paths: &Paths) -> std::path::PathBuf {
    paths.data.join("carddav-state.json")
}

/// Keep a book's cards in the raw store if they changed since the last time. -> whether anything was written.
pub fn keep(paths: &Paths, url: &str, cards: &[Card]) -> std::io::Result<bool> {
    let digest = hex::encode(Sha256::digest(format!("{cards:?}").as_bytes()));
    let mut state: BTreeMap<String, String> =
        std::fs::read_to_string(state_file(paths)).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    let key = origin(url);
    if state.get(&key) == Some(&digest) {
        return Ok(false);
    }
    let snap = chrono::Local::now().timestamp_millis();
    RawStore::new(&paths.data).store("contacts", &lines(cards, &key, snap).join("\n"), "personal")?;
    state.insert(key, digest);
    let text = serde_json::to_string_pretty(&state).unwrap_or_default();
    crate::fsutil::write_private_atomic(&state_file(paths), text.as_bytes())?;
    Ok(true)
}

fn propfind(url: &str, acct: &Account, props: &str, depth: &str) -> Result<String, DavError> {
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><d:propfind xmlns:d=\"DAV:\" xmlns:card=\"{CARDDAV}\"><d:prop>{props}</d:prop></d:propfind>"
    );
    dav("PROPFIND", url, acct, depth, body, 60)
}

/// CardDAV discovery: principal -> address book home -> books, all on `base`'s server. -> [(name, url)]
pub fn list_books(base: &str, acct: &Account) -> Result<Vec<(String, String)>, String> {
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
    let starts = [Some(base.to_string()), join(base, "/.well-known/carddav"), join(base, "/")];
    for start in starts.into_iter().flatten() {
        let Some(xml) = step("discover", &start, "<d:current-user-principal/><card:addressbook-home-set/>", "0") else {
            continue;
        };
        let mut at = start.clone();
        let mut h = find(&xml, CARDDAV, "addressbook-home-set");
        if h.is_none()
            && let Some(p) = find(&xml, DAV, "current-user-principal").and_then(|p| join(&start, &p))
        {
            at = p;
            h = step("principal", &at, "<card:addressbook-home-set/>", "0")
                .and_then(|x| find(&x, CARDDAV, "addressbook-home-set"));
        }
        if let Some(h) = h.and_then(|h| join(&at, &h)) {
            home = Some(h);
            break;
        }
    }
    let home = home.ok_or("이 서버에서 주소록을 찾지 못했습니다(CardDAV를 지원하지 않을 수 있습니다)")?;
    let xml = step("list", &home, "<d:displayname/><d:resourcetype/>", "1").ok_or("주소록 목록을 읽지 못했습니다")?;
    let doc = roxmltree::Document::parse(&xml).map_err(|_| "주소록 목록이 XML이 아닙니다")?;
    let mut out = Vec::new();
    for r in doc.root_element().children().filter(|n| n.has_tag_name((DAV, "response"))) {
        let is_book = r.descendants().any(|n| {
            n.has_tag_name((DAV, "resourcetype")) && n.children().any(|c| c.has_tag_name((CARDDAV, "addressbook")))
        });
        let href = r.children().find(|n| n.has_tag_name((DAV, "href"))).and_then(|n| n.text()).unwrap_or("");
        let (true, Some(url)) = (is_book, join(&home, href.trim())) else { continue };
        let name = r
            .descendants()
            .find(|n| n.has_tag_name((DAV, "displayname")))
            .and_then(|n| n.text())
            .unwrap_or("(이름 없음)");
        out.push((name.to_string(), url));
    }
    Ok(out)
}

/// Every card of a book (addressbook-query with no filter).
pub fn fetch(url: &str, acct: &Account) -> Result<Vec<Card>, DavError> {
    let body = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?><card:addressbook-query xmlns:d=\"DAV:\" xmlns:card=\"{CARDDAV}\"><d:prop><d:getetag/><card:address-data/></d:prop></card:addressbook-query>"
    );
    let xml = dav("REPORT", url, acct, "1", body, 120)?;
    let doc = roxmltree::Document::parse(&xml).map_err(|_| DavError::Other("ParseError".into()))?;
    let text: String = doc
        .descendants()
        .filter(|n| n.has_tag_name((CARDDAV, "address-data")))
        .map(|n| n.children().filter(|c| c.is_text()).map(|c| c.text().unwrap_or("")).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n");
    Ok(parse_vcards(&text))
}
