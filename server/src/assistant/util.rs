//! Small readers shared by the assistant: values as they appear in loosely typed records, note frontmatter, times.
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{Local, NaiveDateTime, TimeZone};
use serde_json::Value;

/// A record value as text the way the Python server printed it into prompts: strings as they are, a missing value or
/// null as `None`, booleans as `True`/`False`, anything else as its JSON.
pub fn pystr(v: Option<&Value>) -> String {
    match v {
        None | Some(Value::Null) => "None".into(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => if *b { "True" } else { "False" }.into(),
        Some(other) => other.to_string(),
    }
}

/// Is the value "truthy" (present, not null, not false, not 0, not an empty string, list or map)?
pub fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().map(|f| f != 0.0).unwrap_or(true),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// A record value as a whole number: integers, finite floats (cut toward zero) and strings of digits.
pub fn as_int(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Number(n) => {
            n.as_i64().or_else(|| n.as_f64().filter(|f| f.is_finite() && f.abs() < 9.2e18).map(|f| f.trunc() as i64))
        }
        Value::String(s) => s.trim().parse().ok(),
        _ => None,
    }
}

/// The first `n` characters (not bytes).
pub fn take(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Local time of epoch milliseconds, if it is a time at all.
pub fn local(ts_ms: i64) -> Option<NaiveDateTime> {
    Local.timestamp_millis_opt(ts_ms).single().map(|t| t.naive_local())
}

/// `HH:MM` of epoch milliseconds (`--:--` for a value that is not a time).
pub fn hm(ts_ms: Option<i64>) -> String {
    ts_ms.and_then(local).map(|t| t.format("%H:%M").to_string()).unwrap_or_else(|| "--:--".into())
}

/// Frontmatter values of a note: `key: value` lines in its first 3000 characters, trimmed and without double quotes.
/// A missing key is "".
pub fn front(path: &Path, keys: &[&str]) -> Vec<String> {
    let head = crate::note::head(path, 3000);
    keys.iter().map(|k| front_value(&head, k)).collect()
}

/// The value of the first `key:` line (trimmed, without surrounding double quotes), else "".
pub fn front_value(head: &str, key: &str) -> String {
    crate::note::field(head, key).unwrap_or_default().trim_matches('"').to_string()
}

/// Files in `dir` whose names start with `prefix` and end with `suffix`, sorted by name.
pub fn files_like(dir: &Path, prefix: &str, suffix: &str) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.file_name()
                            .and_then(|n| n.to_str())
                            .map(|n| n.starts_with(prefix) && n.ends_with(suffix))
                            .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    out.sort();
    out
}

/// Files `<dir>/*/<prefix>*<suffix>` (one level of sub-directories, like `Mail/<year>/`), sorted.
pub fn files_like_nested(dir: &Path, prefix: &str, suffix: &str) -> Vec<PathBuf> {
    let mut subs: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| rd.filter_map(Result::ok).map(|e| e.path()).filter(|p| p.is_dir()).collect())
        .unwrap_or_default();
    subs.sort();
    subs.iter().flat_map(|s| files_like(s, prefix, suffix)).collect()
}

/// Counts in first-seen order, listed most common first (ties keep first-seen order), like Python's Counter.
pub struct Counter<K> {
    items: Vec<(K, usize)>,
}

impl<K> Default for Counter<K> {
    fn default() -> Self {
        Counter { items: Vec::new() }
    }
}

impl<K: PartialEq + Clone> Counter<K> {
    pub fn add(&mut self, key: K) {
        match self.items.iter_mut().find(|(k, _)| *k == key) {
            Some((_, n)) => *n += 1,
            None => self.items.push((key, 1)),
        }
    }

    pub fn most_common(&self, n: usize) -> Vec<(K, usize)> {
        let mut v = self.items.clone();
        v.sort_by_key(|item| std::cmp::Reverse(item.1));
        v.truncate(n);
        v
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn loosely_typed_values() {
        assert_eq!(pystr(None), "None");
        assert_eq!(pystr(Some(&json!("a"))), "a");
        assert_eq!(pystr(Some(&json!(3))), "3");
        assert_eq!(as_int(Some(&json!("1792000000000"))), Some(1_792_000_000_000));
        assert_eq!(as_int(Some(&json!(1.9))), Some(1));
        assert_eq!(as_int(Some(&json!(1e300))), None);
        assert_eq!(as_int(Some(&json!([1]))), None);
        assert!(!truthy(Some(&json!(""))));
        assert!(truthy(Some(&json!("x"))));
    }

    #[test]
    fn frontmatter_values_are_trimmed_and_unquoted() {
        let head = "---\ntitle: \"회의\"\nstart:\t2026-01-01T10:00:00\nempty:\n---\nbody title: x\n";
        assert_eq!(front_value(head, "title"), "회의");
        assert_eq!(front_value(head, "start"), "2026-01-01T10:00:00");
        assert_eq!(front_value(head, "empty"), "");
        assert_eq!(front_value(head, "missing"), "");
    }

    #[test]
    fn most_common_keeps_first_seen_order_on_ties() {
        let mut c = Counter::<String>::default();
        for k in ["b", "a", "b", "c", "a"] {
            c.add(k.into());
        }
        let want: Vec<(String, usize)> = vec![("b".into(), 2), ("a".into(), 2), ("c".into(), 1)];
        assert_eq!(c.most_common(10), want);
    }
}
