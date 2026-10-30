//! What `life` needs besides running the collectors: folder names as people read them, which folders to offer first,
//! the jobs of a sync, and the run results kept between runs.
use std::path::Path;

use base64::Engine;
use chrono::Local;
use serde_json::{Value, json};

/// An IMAP folder name (modified UTF-7) for display. Stored values keep the server's raw name.
pub fn imap_utf7_decode(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let Some(j) = after.find('-') else {
            out.push_str(&rest[i..]);
            return out;
        };
        let b = &after[..j];
        if b.is_empty() {
            out.push('&');
        } else {
            let b64 = b.replace(',', "/");
            let bytes =
                base64::engine::general_purpose::STANDARD_NO_PAD.decode(b64.trim_end_matches('=')).unwrap_or_default();
            let units: Vec<u16> = bytes.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
            out.push_str(&String::from_utf16_lossy(&units));
        }
        rest = &after[j + 1..];
    }
    out.push_str(rest);
    out
}

/// INBOX plus the sent folder (by its special-use flag if any, else by common names).
pub fn default_folders(folders: &[(String, Vec<String>)]) -> Vec<String> {
    let mut picked: Vec<String> =
        folders.iter().filter(|(n, _)| n.eq_ignore_ascii_case("INBOX")).map(|(n, _)| n.clone()).collect();
    let flagged: Vec<&String> =
        folders.iter().filter(|(_, f)| f.iter().any(|x| x == "\\sent")).map(|(n, _)| n).collect();
    let sent: Vec<&String> = if flagged.is_empty() {
        folders
            .iter()
            .filter(|(n, _)| matches!(n.to_lowercase().as_str(), "sent" | "sent messages" | "sent items"))
            .map(|(n, _)| n)
            .collect()
    } else {
        flagged
    };
    picked.extend(sent.into_iter().take(1).cloned());
    picked
}

/// The names `life status` reports, in order.
pub fn configured(sources: &toml::Table) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    if sources.contains_key("mail") {
        out.extend(["mail", "files"]);
    }
    if sources.contains_key("calendar") {
        out.push("calendar");
    }
    if !out.is_empty() {
        out.push("librarian"); // sync indexes whatever it collected
    }
    out
}

pub fn load_status(path: &Path) -> serde_json::Map<String, Value> {
    std::fs::read_to_string(path).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// Record one job's run: when, whether it worked, and its last line of output.
pub fn record(path: &Path, name: &str, ok: bool, detail: &str) -> std::io::Result<()> {
    let mut st = load_status(path);
    let now = Local::now().format("%Y-%m-%dT%H:%M:%S%:z").to_string();
    let job = st.entry(name.to_string()).or_insert_with(|| json!({}));
    job["last_run"] = json!(now);
    job["ok"] = json!(ok);
    job["detail"] = json!(detail);
    if ok {
        job["last_ok"] = json!(now);
    }
    crate::fsutil::write_private_atomic(
        path,
        serde_json::to_string_pretty(&Value::Object(st)).expect("JSON").as_bytes(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_names_decode_for_display() {
        let enc = format!(
            "&{}-",
            base64::engine::general_purpose::STANDARD_NO_PAD
                .encode("보관함".encode_utf16().flat_map(u16::to_be_bytes).collect::<Vec<u8>>())
                .replace('/', ",")
        );
        assert_eq!(imap_utf7_decode("INBOX"), "INBOX");
        assert_eq!(imap_utf7_decode("&-"), "&");
        assert_eq!(imap_utf7_decode(&enc), "보관함");
        assert_eq!(imap_utf7_decode(&format!("a{enc}b")), "a보관함b");
    }

    #[test]
    fn the_default_folders_are_the_inbox_and_the_sent_folder() {
        let f = |n: &str, flags: &[&str]| (n.to_string(), flags.iter().map(|s| s.to_string()).collect());
        assert_eq!(
            default_folders(&[f("INBOX", &[]), f("Sent Messages", &["\\sent"]), f("Trash", &["\\trash"])]),
            ["INBOX", "Sent Messages"]
        );
        assert_eq!(default_folders(&[f("INBOX", &[]), f("Sent", &[])]), ["INBOX", "Sent"]);
    }
}
