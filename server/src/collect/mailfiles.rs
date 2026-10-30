//! Keep the files that went through the user's mail, sent and received: the originals, as they were.
//!
//! Mail notes only list attachment names, so the file itself is kept here:
//!   `<data>/raw/files/<sha256[:2]>/<sha256>`   the bytes, untouched (named by their content hash)
//!   `<data>/raw/mailfile/<YYYY-MM-DD>.ndjson`  one line per attachment: where and when it was seen
//! Nothing is converted, summarised or tagged, and who it went to or came from is not copied (that is in the mail
//! note). The same file seen again (a resend, a forward) is one blob and a line per sighting.
use std::sync::LazyLock;

use chrono::Local;
use mail_parser::Message;
use regex::Regex;
use serde_json::json;

use super::mail::{attachment_name, attachment_parts, date, mime_of, norm_id, part_bytes};
use crate::store::{RawStore, put_blob};

/// Does the folder's name say it holds sent mail (sent / 보낸 / 발신)?
pub fn is_sent_folder(name: &str) -> bool {
    static SENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)sent|보낸|발신").unwrap());
    SENT.is_match(name)
}

/// Store the attachments of one mail (`direction`: sent or received). -> (files seen, bytes newly written)
pub fn keep(
    store: &RawStore,
    msg: &Message,
    source: &str,
    direction: &str,
    cls: &str,
) -> std::io::Result<(usize, usize)> {
    let parts = attachment_parts(msg);
    if parts.is_empty() {
        return Ok((0, 0));
    }
    let mail_id = norm_id(msg.message_id().unwrap_or(""));
    let when = date(msg).unwrap_or_else(Local::now).timestamp_millis(); // a mail with no usable date is still kept
    let mut lines = Vec::new();
    let mut written = 0;
    for p in &parts {
        let data = part_bytes(msg, p);
        let (sha, new) = put_blob(store.data(), &data)?;
        if new {
            written += data.len();
        }
        let rec = json!({"type": "file", "ts": when, "sha256": sha, "name": attachment_name(p), "mime": mime_of(p),
                         "size": data.len(), "mail_id": mail_id, "mail_source": source, "direction": direction});
        lines.push(rec.to_string());
    }
    store.store("mailfile", &(lines.join("\n") + "\n"), cls)?;
    Ok((parts.len(), written))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine;
    use serde_json::Value;

    const PDF: &[u8] = b"%PDF-1.4 quote v1";
    const XLSX: &[u8] = b"PK\x03\x04 spreadsheet";

    /// A mail with these attachments: (file name or None, mime, bytes).
    fn mail(mid: &str, files: &[(Option<&str>, &str, &[u8])]) -> Vec<u8> {
        let mut m = format!(
            "Message-ID: <{mid}>\r\nDate: Tue, 03 Feb 2026 10:00:00 +0900\r\nFrom: me@example.com\r\nTo: client@example.com\r\nSubject: =?utf-8?b?6rKs7KCB7IScIOyGoeu2gA==?=\r\nMIME-Version: 1.0\r\n"
        );
        if files.is_empty() {
            m += "Content-Type: text/plain; charset=utf-8\r\n\r\nbody\r\n";
            return m.into_bytes();
        }
        m += "Content-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n첨부 확인 부탁드립니다\r\n";
        for (name, mime, data) in files {
            let disp = name.map(|n| format!("attachment; filename=\"{n}\"")).unwrap_or_else(|| "attachment".into());
            let b64 = base64::engine::general_purpose::STANDARD.encode(data);
            m += &format!(
                "--b\r\nContent-Type: {mime}\r\nContent-Disposition: {disp}\r\nContent-Transfer-Encoding: base64\r\n\r\n{b64}\r\n"
            );
        }
        m += "--b--\r\n";
        m.into_bytes()
    }

    fn lines(store: &RawStore) -> Vec<Value> {
        let mut out = Vec::new();
        for f in crate::assistant::util::files_like(&store.data().join("raw/mailfile"), "", ".ndjson") {
            out.extend(std::fs::read_to_string(f).unwrap().lines().map(|l| serde_json::from_str::<Value>(l).unwrap()));
        }
        out
    }

    #[test]
    fn attachments_are_kept_under_their_hash_without_who_they_went_to() {
        let dir = tempfile::tempdir().unwrap();
        let store = RawStore::new(dir.path());
        let raw = mail(
            "m1@x",
            &[(Some("견적서.pdf"), "application/pdf", PDF), (Some("표.xlsx"), "application/vnd.ms-excel", XLSX)],
        );
        let msg = super::super::mail::parse(&raw).unwrap();
        assert_eq!(
            keep(&store, &msg, "account/Sent Messages", "sent", "personal").unwrap(),
            (2, PDF.len() + XLSX.len())
        );
        let sha = crate::store::sha256_hex(PDF);
        assert_eq!(std::fs::read(crate::store::blob_path(store.data(), &sha)).unwrap(), PDF);
        let ls = lines(&store);
        assert_eq!(ls.len(), 2);
        assert_eq!(
            (ls[0]["name"].as_str(), ls[0]["sha256"].as_str(), ls[0]["direction"].as_str()),
            (Some("견적서.pdf"), Some(sha.as_str()), Some("sent"))
        );
        assert_eq!(ls[0]["mail_id"], "m1@x");
        assert!(
            !ls.iter().any(|l| l.to_string().contains("client@example.com")),
            "who it went to is in the mail note, not here"
        );
    }

    #[test]
    fn the_same_file_in_two_mails_is_one_blob_and_two_sightings_and_a_mail_read_twice_adds_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let store = RawStore::new(dir.path());
        for mid in ["a@x", "b@x", "a@x"] {
            let raw = mail(mid, &[(Some("q.pdf"), "application/pdf", PDF)]);
            keep(&store, &super::super::mail::parse(&raw).unwrap(), "account/INBOX", "received", "personal").unwrap();
        }
        assert_eq!(std::fs::read_dir(store.data().join("raw/files")).unwrap().count(), 1);
        assert_eq!(lines(&store).len(), 2, "one line per mail it was seen in");
    }

    #[test]
    fn a_mail_without_attachments_stores_nothing_and_an_unnamed_one_gets_a_placeholder() {
        let dir = tempfile::tempdir().unwrap();
        let store = RawStore::new(dir.path());
        assert_eq!(
            keep(&store, &super::super::mail::parse(&mail("n@x", &[])).unwrap(), "s", "received", "personal").unwrap(),
            (0, 0)
        );
        assert!(!store.data().join("raw").exists());
        let raw = mail("u@x", &[(None, "application/octet-stream", b"bytes")]);
        keep(&store, &super::super::mail::parse(&raw).unwrap(), "s", "received", "personal").unwrap();
        assert_eq!(lines(&store)[0]["name"], "(unnamed)");
    }

    #[test]
    fn a_sent_folder_is_known_by_its_name() {
        for n in ["Sent Messages", "보낸편지함", "발신함", "SENT"] {
            assert!(is_sent_folder(n), "{n}");
        }
        assert!(!is_sent_folder("INBOX"));
    }
}
