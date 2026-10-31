//! Address books over CardDAV (#22), the way calendars come in: read-only, https only, the account's password never
//! sent to another server. Each book is kept in the raw store (`raw/contacts/`) as a whole list, only when it changed;
//! the librarian uses the newest list of each book (and of the phone) to know who is who. Only names, mail addresses,
//! phone numbers and the company are kept (no notes, birthdays, photos or postal addresses).
use std::collections::BTreeMap;

use serde_json::json;
use sha2::{Digest, Sha256};

use super::calendar::{Account, DavError, check_url, dav, same_server, shown};
use crate::config::Paths;
use crate::store::RawStore;

const DAV: &str = "DAV:";
const CARDDAV: &str = "urn:ietf:params:xml:ns:carddav";

/// What is kept of a vCard.
#[derive(Clone, Debug, PartialEq)]
pub struct Card {
    pub name: String,
    pub org: String,
    /// lower case
    pub emails: Vec<String>,
    pub phones: Vec<String>,
}

/// vCard text unescaped (`\,` `\;` `\n`).
fn unescape(v: &str) -> String {
    v.replace("\\n", " ").replace("\\N", " ").replace("\\,", ",").replace("\\;", ";").replace("\\\\", "\\")
}

/// The cards of a text holding one or more vCards (3.0 or 4.0). Cards with no mail address and no phone number are
/// left out: there is nothing to know them by.
pub fn parse_vcards(text: &str) -> Vec<Card> {
    // unfold: a line starting with a space or tab continues the one before
    let unfolded = text.replace("\r\n", "\n").replace("\n ", "").replace("\n\t", "");
    let mut out = Vec::new();
    let mut cur: Option<(String, String, Card)> = None; // (FN, N as a name, card)
    for line in unfolded.lines() {
        let Some((head, value)) = line.split_once(':') else { continue };
        let prop = head.split(';').next().unwrap_or("").rsplit('.').next().unwrap_or("").to_ascii_uppercase();
        match prop.as_str() {
            "BEGIN" if value.eq_ignore_ascii_case("VCARD") => {
                cur = Some((
                    String::new(),
                    String::new(),
                    Card { name: String::new(), org: String::new(), emails: vec![], phones: vec![] },
                ));
            }
            "END" if value.eq_ignore_ascii_case("VCARD") => {
                if let Some((fname, nname, mut c)) = cur.take() {
                    c.name = if fname.trim().is_empty() { nname } else { fname.trim().to_string() };
                    c.name = c.name.chars().take(100).collect();
                    if !c.emails.is_empty() || !c.phones.is_empty() {
                        out.push(c);
                    }
                }
            }
            _ => {
                let Some((fname, nname, c)) = cur.as_mut() else { continue };
                let value = value.trim();
                match prop.as_str() {
                    "FN" => *fname = unescape(value),
                    "N" => {
                        let parts: Vec<String> = value.split(';').map(unescape).collect();
                        let (family, given) =
                            (parts.first().cloned().unwrap_or_default(), parts.get(1).cloned().unwrap_or_default());
                        let hangul = family.chars().chain(given.chars()).any(|ch| ('가'..='힣').contains(&ch));
                        *nname = if hangul { format!("{family}{given}") } else { format!("{given} {family}") }
                            .trim()
                            .to_string();
                    }
                    "ORG" if c.org.is_empty() => {
                        c.org = unescape(value.split(';').next().unwrap_or("")).chars().take(100).collect()
                    }
                    "EMAIL" => {
                        let e = value.trim_start_matches("mailto:").to_lowercase();
                        if e.contains('@') && e.len() <= 254 && c.emails.len() < 20 && !c.emails.contains(&e) {
                            c.emails.push(e);
                        }
                    }
                    "TEL" => {
                        let t = value.trim_start_matches("tel:").to_string();
                        if !t.is_empty() && t.len() <= 40 && c.phones.len() < 20 && !c.phones.contains(&t) {
                            c.phones.push(t);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    out
}

/// The raw store lines of a book: one per card, all with the same `snap` and `origin` (whose list it is).
pub fn lines(cards: &[Card], origin: &str, snap: i64) -> Vec<String> {
    let line = |c: &Card| {
        json!({"type": "contact", "snap": snap, "ts": snap, "origin": origin, "name": c.name, "org": c.org,
               "emails": c.emails, "phones": c.phones})
        .to_string()
    };
    if cards.is_empty() {
        // an empty book is kept too (one marker line), so its deleted cards leave the index
        return vec![json!({"type": "contact", "snap": snap, "ts": snap, "origin": origin, "name": "", "emails": [], "phones": []}).to_string()];
    }
    cards.iter().map(line).collect()
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
