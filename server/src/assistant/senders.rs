//! Which messenger senders may feed the diary: a list the user chooses, default none.
use std::collections::BTreeSet;
use std::fs;

use chrono::Local;
use serde_json::{Value, json};

use super::NOISE_PKGS;
use super::util::{Counter, pystr, truthy};
use crate::config::Paths;
use crate::fsutil::write_private_atomic;

/// The entries the user allowed: `"<app>: <sender>"` or `"<app>: *"`. A missing or unreadable list allows nothing.
pub fn load_allow(paths: &Paths) -> BTreeSet<String> {
    let Ok(text) = fs::read_to_string(&paths.allow_file) else {
        return BTreeSet::new();
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Object(o)) => match o.get("allow") {
            Some(Value::Array(a)) => a.iter().map(|v| pystr(Some(v))).collect(),
            _ => BTreeSet::new(),
        },
        _ => BTreeSet::new(),
    }
}

pub fn save_allow(paths: &Paths, items: &[Value]) -> std::io::Result<()> {
    let set: BTreeSet<String> = items.iter().map(|v| pystr(Some(v))).collect();
    let text = serde_json::to_string_pretty(&json!({"allow": set})).expect("a list of strings serializes");
    write_private_atomic(&paths.allow_file, text.as_bytes())
}

/// May a notification from this app and sender feed the diary?
pub fn is_allowed(allow: &BTreeSet<String>, label: &str, title: &str) -> bool {
    allow.contains(&format!("{label}: *")) || allow.contains(&format!("{label}: {title}"))
}

/// The same for a record's raw `label` and `title` values.
pub fn record_allowed(allow: &BTreeSet<String>, rec: &serde_json::Map<String, Value>) -> bool {
    is_allowed(allow, &pystr(rec.get("label")), &pystr(rec.get("title")))
}

/// Who sent notifications in the last `days` days: `[{label, title, count, allowed}]`, most active first.
pub fn list_senders(paths: &Paths, days: i64) -> Vec<Value> {
    let allow = load_allow(paths);
    let mut counts: Counter<(Value, Value)> = Counter::default();
    let today = Local::now().date_naive();
    for i in 0..days {
        let day = today - chrono::Duration::days(i);
        let path = paths.raw().join("lorelog").join(format!("{}.ndjson", day.format("%Y-%m-%d")));
        let Ok(bytes) = fs::read(&path) else { continue };
        for line in String::from_utf8_lossy(&bytes).lines() {
            let Ok(Value::Object(d)) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            if d.get("type").and_then(Value::as_str) == Some("notification") && !noise(&d) && truthy(d.get("title")) {
                counts.add((
                    d.get("label").cloned().unwrap_or(Value::Null),
                    d.get("title").cloned().unwrap_or(Value::Null),
                ));
            }
        }
    }
    counts
        .most_common(usize::MAX)
        .into_iter()
        .map(|((label, title), n)| {
            let allowed = is_allowed(&allow, &pystr(Some(&label)), &pystr(Some(&title)));
            json!({"label": label, "title": title, "count": n, "allowed": allowed})
        })
        .collect()
}

/// Is this a phone system app whose events say nothing about the user's day?
pub fn noise(rec: &serde_json::Map<String, Value>) -> bool {
    rec.get("pkg").and_then(Value::as_str).map(|p| NOISE_PKGS.contains(&p)).unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_allowed_by_default() {
        assert!(!is_allowed(&BTreeSet::new(), "카카오톡", "철수"));
    }

    #[test]
    fn app_wide_and_per_sender_entries() {
        let allow: BTreeSet<String> = ["WORKS: *", "카카오톡: 철수"].iter().map(|s| s.to_string()).collect();
        assert!(is_allowed(&allow, "WORKS", "아무나"));
        assert!(is_allowed(&allow, "카카오톡", "철수"));
        assert!(!is_allowed(&allow, "카카오톡", "엄마"));
    }

    #[test]
    fn the_list_round_trips_sorted_and_without_repeats() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        save_allow(&paths, &[json!("카카오톡: 홍길동"), json!("앱: *"), json!("앱: *")]).unwrap();
        assert_eq!(load_allow(&paths).into_iter().collect::<Vec<_>>(), ["앱: *", "카카오톡: 홍길동"]);
    }
}
