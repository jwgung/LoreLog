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
