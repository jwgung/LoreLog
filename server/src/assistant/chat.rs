//! The assistant's chat log: `raw/diary/<day>.ndjson` lines of `type: chat`, the user's and the assistant's turns.
use std::fs;

use chrono::Local;
use serde_json::{Map, Value, json};

use super::context::read_records;
use super::diary::split_correction;
use super::util::pystr;
use crate::config::Paths;
use crate::fsutil::append_private;

/// The day's chat turns in order.
pub fn chat_log(paths: &Paths, day: &str) -> Vec<Map<String, Value>> {
    read_records(
        &paths.raw().join("diary").join(format!("{day}.ndjson")),
        None,
    )
    .unwrap_or_default()
    .into_iter()
    .filter(|d| d.get("type").and_then(Value::as_str) == Some("chat"))
    .collect()
}

pub fn append_chat(paths: &Paths, day: &str, role: &str, text: &str) -> std::io::Result<()> {
    let raw = paths.raw().join("diary");
    fs::create_dir_all(&raw)?;
    let rec = json!({"type": "chat", "ts": Local::now().timestamp_millis(), "date": day, "role": role, "text": text,
                     "_src": "diary", "_class": "personal"});
    append_private(&raw.join(format!("{day}.ndjson")), &format!("{rec}\n"))
}

/// Today's exchange for the app to show again: `[{role, text, wrong?}]`. A correction is stored with a wrapper line;
/// it is unwrapped.
pub fn chat_history(paths: &Paths, day: &str) -> Vec<Value> {
    chat_log(paths, day)
        .iter()
        .map(|t| {
            let role = pystr(t.get("role"));
            let text = pystr(t.get("text"));
            match (role == "user").then(|| split_correction(&text)).flatten() {
                Some((wrong, said)) => json!({"role": role, "text": said, "wrong": wrong}),
                None => json!({"role": role, "text": text}),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_correction_wrapper_is_unwrapped() {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::under(dir.path());
        append_chat(
            &p,
            "2026-01-02",
            "user",
            "(정리의 이 문장이 틀렸다고 알려 주셨어요: \"10초 통화\")\n20분 했어",
        )
        .unwrap();
        append_chat(&p, "2026-01-02", "assistant", "알겠어요.").unwrap();
        let h = chat_history(&p, "2026-01-02");
        assert_eq!(
            h[0],
            json!({"role": "user", "text": "20분 했어", "wrong": "10초 통화"})
        );
        assert_eq!(h[1], json!({"role": "assistant", "text": "알겠어요."}));
    }
}
