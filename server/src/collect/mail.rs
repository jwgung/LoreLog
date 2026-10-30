//! Mail as the vault keeps it: a parsed mail becomes a `Mail/<year>/` note with the frontmatter of the Mailstone notes,
//! and its attachments become blobs in the raw store. Shared by mail-sync and mail-files.
//!
//! Which parts are the body and which are attachments follows Python's `email` package, which the notes were made
//! with: the body is the first text/plain (else text/html) reachable without entering attachments or the non-start
//! parts of multipart/related; the attachments are the immediate sub-parts of the message that are not its first
//! plain, html, related or alternative part (unless marked as attachments).
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use base64::Engine;
use chrono::{DateTime, Local, TimeZone};
use mail_parser::{Address, Encoding, HeaderValue, Message, MessageParser, MessagePart, MimeHeaders, PartType};
use regex::Regex;
use sha1::{Digest, Sha1};

use crate::assistant::util::files_like_nested;

pub fn parse(raw: &[u8]) -> Option<Message<'_>> {
    MessageParser::default().parse(raw)
}

/// `<id>` -> `id`.
pub fn norm_id(v: &str) -> String {
    v.trim().trim_matches(['<', '>']).trim().to_string()
}

fn header_ids(v: &HeaderValue) -> Vec<String> {
    match v {
        HeaderValue::Text(t) => vec![t.to_string()],
        HeaderValue::TextList(l) => l.iter().map(|t| t.to_string()).collect(),
        _ => Vec::new(),
    }
}

/// When the mail says it was sent, in local time.
pub fn date(msg: &Message) -> Option<DateTime<Local>> {
    msg.date().and_then(|d| Local.timestamp_opt(d.to_timestamp(), 0).single())
}

/// The addresses of a header as `name <addr>` (or `<addr>`), those without an address left out.
pub fn addrs(a: Option<&Address>) -> Vec<String> {
    let list: Vec<&mail_parser::Addr> = match a {
        Some(Address::List(l)) => l.iter().collect(),
        Some(Address::Group(g)) => g.iter().flat_map(|g| g.addresses.iter()).collect(),
        None => Vec::new(),
    };
    list.iter()
        .filter_map(|x| {
            let addr = x.address.as_deref().filter(|s| !s.is_empty())?;
            // one line: a decoded line break in a name must not start a new frontmatter key
            let name =
                x.name.as_deref().map(|n| n.split_whitespace().collect::<Vec<_>>().join(" ")).unwrap_or_default();
            let addr = addr.split_whitespace().collect::<String>();
            Some(if name.is_empty() { format!("<{addr}>") } else { format!("{name} <{addr}>") })
        })
        .collect()
}

/// The first address of a header (lower case is up to the caller), or "".
pub fn first_addr(a: Option<&Address>) -> String {
    match a {
        Some(Address::List(l)) => l.first().and_then(|x| x.address.as_deref()).unwrap_or("").to_string(),
        Some(Address::Group(g)) => g
            .iter()
            .flat_map(|g| g.addresses.iter())
            .next()
            .and_then(|x| x.address.as_deref())
            .unwrap_or("")
            .to_string(),
        None => String::new(),
    }
}

/// A fingerprint that recognises a mail whose note was stored under another id scheme (Mailstone's internal ids).
pub fn fingerprint(day: &str, hm: &str, addr: &str, subject: &str) -> String {
    static NOISE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"[\s"\\]+"#).unwrap());
    format!("{day}|{hm}|{}|{}", addr.trim().to_lowercase(), NOISE.replace_all(subject, "").to_lowercase())
}

pub fn msg_fingerprint(msg: &Message) -> String {
    let Some(d) = date(msg) else { return String::new() };
    fingerprint(
        &d.format("%Y-%m-%d").to_string(),
        &d.format("%H:%M").to_string(),
        &first_addr(msg.from()),
        &subject(msg),
    )
}

/// The subject as a person reads it. Encoded words are decoded here by RFC 2047's rule (whitespace between two encoded
/// words is not text), which is also how the notes were made; the parser's own reading keeps such spaces.
pub fn subject(msg: &Message) -> String {
    match msg.header_raw("Subject") {
        Some(raw) if raw.contains("=?") => decode_words(raw).trim().to_string(),
        _ => msg.subject().unwrap_or("").trim().to_string(),
    }
}

/// An unstructured header value with RFC 2047 encoded words decoded and its folding undone.
pub fn decode_words(raw: &str) -> String {
    static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"=\?([^?\s]+)\?([BbQq])\?([^?\s]*)\?=").unwrap());
    static FOLD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\r?\n([ \t])").unwrap());
    let raw = FOLD.replace_all(raw.trim_end_matches(['\r', '\n']), "$1");
    let mut out = String::new();
    let mut last = 0;
    let mut after_word = false;
    for m in WORD.captures_iter(&raw) {
        let all = m.get(0).expect("whole match");
        let gap = &raw[last..all.start()];
        if !(after_word && gap.chars().all(char::is_whitespace)) {
            out.push_str(gap);
        }
        let charset = m[1].split('*').next().unwrap_or("utf-8");
        let bytes = if m[2].eq_ignore_ascii_case("b") {
            base64::engine::general_purpose::STANDARD
                .decode(m[3].trim_end_matches('='))
                .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(m[3].trim_end_matches('=')))
                .ok()
        } else {
            Some(quoted_printable(m[3].replace('_', " ").as_bytes()))
        };
        match (bytes, encoding_rs::Encoding::for_label(charset.as_bytes())) {
            (Some(b), Some(enc)) => out.push_str(&enc.decode(&b).0),
            _ => out.push_str(all.as_str()),
        }
        last = all.end();
        after_word = true;
    }
    out.push_str(&raw[last..]);
    out
}

fn yq(s: &str) -> String {
    static WS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    let s = WS.replace_all(s, " ").trim().to_string();
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn slug(subject: &str) -> String {
    static NON: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9]+").unwrap());
    let s = NON.replace_all(&subject.to_lowercase(), "-").trim_matches('-').to_string();
    let s: String = s.chars().take(60).collect();
    let s = s.trim_matches('-').to_string();
    if s.is_empty() { "mail".into() } else { s }
}

// ---------- parts ----------

fn ctype(p: &MessagePart) -> (String, String) {
    match p.content_type() {
        Some(c) => (c.ctype().to_lowercase(), c.subtype().unwrap_or("").to_lowercase()),
        None => ("text".into(), "plain".into()),
    }
}

fn is_attachment(p: &MessagePart) -> bool {
    p.content_disposition().map(|d| d.ctype().eq_ignore_ascii_case("attachment")).unwrap_or(false)
}

fn children<'a>(msg: &'a Message<'a>, p: &'a MessagePart<'a>) -> Vec<&'a MessagePart<'a>> {
    match &p.body {
        PartType::Multipart(ids) => ids.iter().filter_map(|i| msg.part(*i)).collect(),
        _ => Vec::new(),
    }
}

/// The start part of a multipart/related: the one its `start` names, else the first.
fn related_start<'a>(msg: &'a Message<'a>, p: &'a MessagePart<'a>) -> Option<&'a MessagePart<'a>> {
    let kids = children(msg, p);
    let start = p.content_type().and_then(|c| c.attribute("start")).map(|s| s.trim().to_string());
    if let Some(s) = start {
        let found = kids.iter().find(|k| {
            k.content_id().map(|id| format!("<{id}>") == s || id == s.trim_matches(['<', '>'])).unwrap_or(false)
        });
        if let Some(f) = found {
            return Some(f);
        }
    }
    kids.first().copied()
}

fn find_body<'a>(msg: &'a Message<'a>, p: &'a MessagePart<'a>, out: &mut Vec<(usize, &'a MessagePart<'a>)>) {
    if is_attachment(p) {
        return;
    }
    let (main, sub) = ctype(p);
    if main == "text" {
        if let Some(prio) = ["plain", "html"].iter().position(|s| *s == sub) {
            out.push((prio, p));
        }
        return;
    }
    if main != "multipart" {
        return;
    }
    if sub != "related" {
        for k in children(msg, p) {
            find_body(msg, k, out);
        }
        return;
    }
    if let Some(c) = related_start(msg, p) {
        find_body(msg, c, out);
    }
}

/// The body as text: a plain part as it is (trimmed), an html part as text.
pub fn body_text(msg: &Message) -> String {
    let Some(root) = msg.part(0) else { return String::new() };
    let mut found = Vec::new();
    find_body(msg, root, &mut found);
    let Some(best) = found.iter().min_by_key(|(prio, _)| *prio).map(|(_, p)| *p) else { return String::new() };
    let text = match &best.body {
        PartType::Text(t) | PartType::Html(t) => t.to_string(),
        _ => String::new(),
    };
    if ctype(best).1 == "plain" { text.trim().to_string() } else { html_to_text(&text) }
}

/// The attachments: the immediate sub-parts of the message that are not body candidates.
pub fn attachment_parts<'a>(msg: &'a Message<'a>) -> Vec<&'a MessagePart<'a>> {
    let Some(root) = msg.part(0) else { return Vec::new() };
    let (main, sub) = ctype(root);
    if main != "multipart" || sub == "alternative" {
        return Vec::new();
    }
    let kids = children(msg, root);
    if sub == "related" {
        let start = related_start(msg, root).map(|p| p as *const _);
        return kids.into_iter().filter(|k| Some(*k as *const _) != start).collect();
    }
    let mut seen = HashSet::new();
    kids.into_iter()
        .filter(|k| {
            let (m, s) = ctype(k);
            let body_type = matches!(
                (m.as_str(), s.as_str()),
                ("text", "plain") | ("text", "html") | ("multipart", "related") | ("multipart", "alternative")
            );
            !(body_type && !is_attachment(k) && seen.insert(s))
        })
        .collect()
}

/// An attachment's file name, or "(unnamed)".
pub fn attachment_name(p: &MessagePart) -> String {
    p.attachment_name().filter(|n| !n.is_empty()).unwrap_or("(unnamed)").to_string()
}

pub fn mime_of(p: &MessagePart) -> String {
    let (m, s) = ctype(p);
    format!("{m}/{s}")
}

/// The bytes of a part as they were before transfer encoding (the user's file exactly, whatever its charset).
pub fn part_bytes(msg: &Message, p: &MessagePart) -> Vec<u8> {
    let raw = msg.raw_message();
    let (a, b) = (p.offset_body as usize, (p.offset_end as usize).min(raw.len()));
    let body = if a <= b { &raw[a..b] } else { &[][..] };
    match p.encoding {
        Encoding::Base64 => {
            let clean: Vec<u8> =
                body.iter().copied().filter(|c| c.is_ascii_alphanumeric() || matches!(c, b'+' | b'/')).collect();
            let engine = base64::engine::GeneralPurpose::new(
                &base64::alphabet::STANDARD,
                base64::engine::GeneralPurposeConfig::new()
                    .with_decode_padding_mode(base64::engine::DecodePaddingMode::Indifferent)
                    .with_decode_allow_trailing_bits(true),
            );
            engine.decode(&clean).unwrap_or_default()
        }
        Encoding::QuotedPrintable => quoted_printable(body),
        Encoding::None => match &p.body {
            PartType::Binary(b) | PartType::InlineBinary(b) => b.to_vec(),
            _ => body.to_vec(),
        },
    }
}

fn quoted_printable(b: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'=' {
            if b.get(i + 1) == Some(&b'\n') {
                i += 2;
                continue;
            }
            if b.get(i + 1) == Some(&b'\r') && b.get(i + 2) == Some(&b'\n') {
                i += 3;
                continue;
            }
            if let (Some(h), Some(l)) = (
                b.get(i + 1).and_then(|c| (*c as char).to_digit(16)),
                b.get(i + 2).and_then(|c| (*c as char).to_digit(16)),
            ) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    out
}

// ---------- html ----------

/// HTML as plain text: script, style and head left out, block elements as line breaks, entities decoded.
pub fn html_to_text(html: &str) -> String {
    const BLOCK: [&str; 13] = ["p", "div", "br", "li", "tr", "h1", "h2", "h3", "h4", "h5", "h6", "table", "blockquote"];
    let block = |t: &str| BLOCK.contains(&t);
    let mut out = String::new();
    let mut skip = 0usize;
    let mut text = String::new();
    let flush = |text: &mut String, out: &mut String, skip: usize| {
        if !text.is_empty() {
            if skip == 0 {
                out.push_str(&html_escape::decode_html_entities(text));
            }
            text.clear();
        }
    };
    let b = html.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'<' {
            let next = html[i..].find('<').map(|n| i + n).unwrap_or(b.len());
            text.push_str(&html[i..next]);
            i = next;
            continue;
        }
        let rest = &html[i..];
        if rest.starts_with("<!--") {
            flush(&mut text, &mut out, skip);
            i += rest.find("-->").map(|n| n + 3).unwrap_or(rest.len());
            continue;
        }
        let after = b.get(i + 1).copied().unwrap_or(0);
        if after == b'!' || after == b'?' {
            flush(&mut text, &mut out, skip);
            i += rest.find('>').map(|n| n + 1).unwrap_or(rest.len());
            continue;
        }
        let closing = after == b'/';
        let name_start = if closing { i + 2 } else { i + 1 };
        if !b.get(name_start).map(|c| c.is_ascii_alphabetic()).unwrap_or(false) {
            text.push('<');
            i += 1;
            continue;
        }
        flush(&mut text, &mut out, skip);
        let name_end = (name_start..b.len())
            .find(|&j| !(b[j].is_ascii_alphanumeric() || b[j] == b'-' || b[j] == b':'))
            .unwrap_or(b.len());
        let name = html[name_start..name_end].to_ascii_lowercase();
        // the end of the tag, past quoted attribute values
        let mut j = name_end;
        let mut quote = 0u8;
        while j < b.len() {
            match (quote, b[j]) {
                (0, b'"') | (0, b'\'') => quote = b[j],
                (q, c) if q != 0 && c == q => quote = 0,
                (0, b'>') => break,
                _ => {}
            }
            j += 1;
        }
        i = (j + 1).min(b.len());
        let skipped = matches!(name.as_str(), "script" | "style" | "head");
        if !closing && j > 0 && j < b.len() && b[j - 1] == b'/' {
            // `<br/>`: a start and an end tag at once (as Python's parser reports it)
            if block(&name) {
                out.push_str("\n\n");
            }
            continue;
        }
        if closing {
            if skipped {
                skip = skip.saturating_sub(1);
            } else if block(&name) {
                out.push('\n');
            }
            continue;
        }
        if skipped {
            skip += 1;
            if name == "script" || name == "style" {
                // raw text up to the closing tag
                let lower = html[i..].to_ascii_lowercase();
                let end = lower.find(&format!("</{name}")).map(|n| i + n).unwrap_or(b.len());
                i = end;
            }
        } else if block(&name) {
            out.push('\n');
        }
    }
    flush(&mut text, &mut out, skip);
    static SPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new("[ \t\u{a0}]+").unwrap());
    static GAPS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n\s*\n\s*\n+").unwrap());
    let t = SPACE.replace_all(&out, " ");
    GAPS.replace_all(&t, "\n\n").trim().to_string()
}

// ---------- notes ----------

/// One mail -> (path in the vault, note text, mail id).
pub fn render(msg: &Message, source: &str, own: &HashSet<String>, synced: &str) -> (String, String, String) {
    let mut mid = norm_id(msg.message_id().unwrap_or(""));
    if mid.is_empty() {
        // no Message-ID: one from the fingerprint (an empty id would be mistaken for "already known")
        mid = format!("nomid-{}", &hex::encode(Sha1::digest(msg_fingerprint(msg).as_bytes()))[..16]);
    }
    let subject = subject(msg);
    let dt = date(msg).unwrap_or_else(Local::now);
    let sender = first_addr(msg.from()).to_lowercase();
    let refs: Vec<String> = header_ids(msg.references()).iter().map(|r| norm_id(r)).filter(|r| !r.is_empty()).collect();
    let in_reply: Vec<String> = header_ids(msg.in_reply_to());
    let atts: Vec<String> = attachment_parts(msg).iter().map(|p| yq(&attachment_name(p))).collect();
    let direction = if !sender.is_empty() && own.contains(&sender) { "ausgehend" } else { "eingehend" };
    let fm = [
        "---".to_string(),
        "type: mail".into(),
        format!("mail_id: {mid}"),
        format!("mail_source: {source}"),
        "mail_state: live".into(),
        format!("mail_synced: {synced}"),
        format!("title: {}", yq(&subject)),
        format!("date: {}", dt.format("%Y-%m-%d")),
        format!("time: {}", dt.format("%H:%M")),
        format!("from: {}", addrs(msg.from()).join(", ")),
        format!("to: [{}]", addrs(msg.to()).join(", ")),
        format!("cc: [{}]", addrs(msg.cc()).join(", ")),
        format!("subject: {}", yq(&subject)),
        format!("in_reply_to: {}", norm_id(&in_reply.join("> <"))),
        format!("references: [{}]", refs.join(", ")),
        format!("attachments: [{}]", atts.join(", ")),
        format!("richtung: {direction}"),
        "---".into(),
    ];
    let note = format!("{}\n\n## Notes\n\n%% mail-sync:begin %%\n## Message\n\n{}\n", fm.join("\n"), body_text(msg));
    let rel = format!("Mail/{}/{}-{}.md", dt.format("%Y"), dt.format("%Y-%m-%d-%H%M"), slug(&subject));
    (rel, note, mid)
}

/// Mails that already have a note -> (mail ids, fingerprints).
pub fn known_notes(vault: &Path) -> (HashSet<String>, HashSet<String>) {
    static ANGLE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<([^>]+)>").unwrap());
    static AT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(\S+@\S+)").unwrap());
    let (mut ids, mut fps) = (HashSet::new(), HashSet::new());
    for p in files_like_nested(&vault.join("Mail"), "", ".md") {
        let text = fs::read(&p).map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let head: String = text.chars().take(1500).collect();
        let get = |k: &str| {
            head.lines().find_map(|l| l.strip_prefix(k).and_then(|r| r.strip_prefix(':')).map(|r| r.trim().to_string()))
        };
        if let Some(id) = get("mail_id").filter(|v| !v.is_empty()) {
            ids.insert(id);
        }
        if let (Some(d), Some(t), Some(s), Some(f)) = (get("date"), get("time"), get("subject"), get("from")) {
            let a = ANGLE.captures(&f).or_else(|| AT.captures(&f)).map(|c| c[1].to_string()).unwrap_or_default();
            fps.insert(fingerprint(&d, &t, &a, &s));
        }
    }
    (ids, fps)
}

/// Write a note, never over another: `-2`, `-3`, ... when the name is taken.
pub fn write_note(vault: &Path, rel: &str, note: &str) -> std::io::Result<PathBuf> {
    let mut f = vault.join(rel);
    let mut n = 2;
    while f.exists() {
        f = vault.join(rel.replace(".md", &format!("-{n}.md")));
        n += 1;
    }
    fs::create_dir_all(f.parent().unwrap_or(vault))?;
    crate::fsutil::write_private_atomic(&f, note.as_bytes())?; // a mail is the user's own: 0600, like every record
    Ok(f)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAW: &[u8] = b"From: =?utf-8?b?7J207ZiE66+8?= <hmlee@x.com>\r\nTo: A B <a@x.com>, c@y.com\r\nCc: =?utf-8?b?6raB7KCV7JuQ?= <jw@x.com>\r\nSubject: =?utf-8?b?UmU6IOyKpOuniO2KuO2MqeuKkOumrCBSRlE=?=\r\nDate: Tue, 27 Oct 2026 14:37:00 +0900\r\nMessage-ID: <abc123@x.com>\r\nIn-Reply-To: <prev@x.com>\r\nReferences: <r1@x.com> <prev@x.com>\r\nMIME-Version: 1.0\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<html><head><style>p{}</style></head><body><p>\xec\x95\x88\xeb\x85\x95\xed\x95\x98\xec\x84\xb8\xec\x9a\x94</p><div>\xea\xb0\x90\xec\x82\xac&nbsp;\xed\x95\xa9\xeb\x8b\x88\xeb\x8b\xa4</div></body></html>";

    #[test]
    fn a_mail_becomes_a_note_like_mailstones() {
        let msg = parse(RAW).unwrap();
        let own: HashSet<String> = ["jw@x.com".to_string()].into();
        let (rel, note, mid) = render(&msg, "account/Sent Messages", &own, "2026-10-27T18:00:00+09:00");
        assert_eq!(mid, "abc123@x.com");
        let local = Local
            .timestamp_opt(
                chrono::DateTime::parse_from_rfc2822("Tue, 27 Oct 2026 14:37:00 +0900").unwrap().timestamp(),
                0,
            )
            .unwrap();
        assert_eq!(rel, format!("Mail/2026/{}-re-rfq.md", local.format("%Y-%m-%d-%H%M")));
        assert!(note.contains("from: 이현민 <hmlee@x.com>\n"), "{note}");
        assert!(note.contains("to: [A B <a@x.com>, <c@y.com>]\n"));
        assert!(note.contains("in_reply_to: prev@x.com\nreferences: [r1@x.com, prev@x.com]\nattachments: []\nricht"));
        assert!(note.contains("richtung: eingehend"), "the sender is not the user");
        assert!(note.ends_with("## Message\n\n안녕하세요\n\n감사 합니다\n"), "{note}");
        let me: HashSet<String> = ["hmlee@x.com".to_string()].into();
        assert!(render(&msg, "x", &me, "t").1.contains("richtung: ausgehend"));
    }

    #[test]
    fn html_becomes_text() {
        assert_eq!(
            html_to_text("<p>a&amp;b</p><script>x<y</script><div>c&nbsp;&nbsp; d</div><br>e"),
            "a&b\n\nc d\n\ne"
        );
        assert_eq!(html_to_text("a.<br/>b"), "a.\n\nb", "a self-closed block tag is a start and an end");
    }

    #[test]
    fn adjacent_encoded_words_join_without_the_space_between_them() {
        assert_eq!(decode_words("=?UTF-8?B?7ZWc6riA?=\r\n =?UTF-8?B?Lg==?="), "한글.");
        assert_eq!(decode_words("Re: =?ks_c_5601-1987?B?x9Gx2w==?= ok"), "Re: 한글 ok");
        assert_eq!(decode_words("=?utf-8?Q?a_b=2E?= plain"), "a b. plain");
    }

    #[test]
    fn attachments_are_the_parts_that_are_not_the_body() {
        let raw = b"Subject: s\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nbody\r\n--b\r\nContent-Type: application/pdf; name=\"a.pdf\"\r\nContent-Disposition: attachment; filename=\"a.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0x\r\n--b\r\nContent-Type: text/plain; charset=euc-kr\r\nContent-Disposition: attachment; filename=\"n.txt\"\r\nContent-Transfer-Encoding: base64\r\n\r\nx8Gx2w==\r\n--b--\r\n";
        let msg = parse(raw).unwrap();
        let parts = attachment_parts(&msg);
        assert_eq!(parts.iter().map(|p| attachment_name(p)).collect::<Vec<_>>(), ["a.pdf", "n.txt"]);
        assert_eq!(part_bytes(&msg, parts[0]), b"%PDF-1");
        assert_eq!(part_bytes(&msg, parts[1]), [0xc7, 0xc1, 0xb1, 0xdb], "a text file keeps its own charset's bytes");
        assert_eq!(body_text(&msg), "body");
    }
}
