//! Measuring the librarian (#7): a file of questions, each with the record it should find, searched the two ways the
//! assistant can (the user's words as they are, and the words the model picks), and the rank the record comes back
//! at. The file holds real names and subjects, so it lives outside the repository, in the vault (see
//! `life-librarian eval`).
//!
//! Kinds of question and what counts as found in the top `k`:
//!   status     "그 건 어떻게 됐지"        the thread it is about
//!   precedent  "전에 어떻게 했지"          the earlier thread it is about
//!   person     "이 사람이랑 뭐 있었지"      any of the latest five things with that address up to today: threads of mail
//!                                          they sent or received (not copied, not mass mail) and events they attended
//!                                          (read from the notes, not from the index), and how many hits are with them
//!   pending    "이번 달에 뭐가 걸려 있지"   not scored: kept to see what is asked
use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use serde::Deserialize;

use super::Librarian;
use crate::assistant::recall;
use crate::assistant::util::files_like_nested;
use crate::llm::Model;
use crate::note;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Status,
    Person,
    Precedent,
    Pending,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Status => "그 건 어떻게 됐지",
            Kind::Person => "이 사람이랑 뭐 있었지",
            Kind::Precedent => "전에 어떻게 했지",
            Kind::Pending => "이번 달에 뭐가 걸려 있지",
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Case {
    pub kind: Kind,
    pub q: String,
    /// status, precedent: the thread (as the index names it, `mail:<message id>`)
    pub thread: Option<String>,
    /// person: their mail address
    pub person: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(rename = "case", default)]
    cases: Vec<Case>,
}

/// The cases of a file. Err: not TOML, an unknown kind, or a case without what it should find.
pub fn load(path: &Path) -> anyhow::Result<Vec<Case>> {
    let text = std::fs::read_to_string(path)?;
    // the message only: toml's own error quotes the line, and the file holds private questions
    let f: File = toml::from_str(&text).map_err(|e| {
        let line = e.span().map(|s| text[..s.start.min(text.len())].lines().count().max(1));
        anyhow::anyhow!("line {}: {}", line.map(|l| l.to_string()).unwrap_or_else(|| "?".into()), e.message())
    })?;
    for (i, c) in f.cases.iter().enumerate() {
        let missing = match c.kind {
            Kind::Status | Kind::Precedent if c.thread.is_none() => Some("thread"),
            Kind::Person if c.person.is_none() => Some("person"),
            _ => None,
        };
        if let Some(what) = missing {
            anyhow::bail!("case {} ({:?}): needs {what}", i + 1, c.kind);
        }
    }
    Ok(f.cases)
}

/// One case measured.
#[derive(Clone, Debug)]
pub struct Result1 {
    pub kind: Kind,
    pub q: String,
    /// the rank (1-based) the record came back at with the user's words; None: not in the top k
    pub raw_rank: Option<usize>,
    /// the words the model picked; None when no model was asked, empty when it chose not to search
    pub model_words: Option<Vec<String>>,
    pub model_rank: Option<usize>,
    /// person: (hits with that person, hits) for the user's words
    pub person_share: Option<(usize, usize)>,
}

/// Does this note's header name the address (from, to or cc)?
fn involves(file: &str, addr: &str) -> bool {
    let head = note::head(Path::new(file), 3000).to_lowercase();
    ["from", "to", "cc"].iter().any(|k| note::field(&head, k).is_some_and(|v| v.contains(addr)))
}

/// Was this mail with that address: sent or received by them, not a mass mail (#22)?
fn with(head: &str, addr: &str) -> bool {
    let people = |k: &str| super::records::addresses(note::field(head, k).unwrap_or(""), "");
    let (to, cc) = (people("to"), people("cc"));
    let direct = people("from").iter().chain(&to).any(|p| p.addr == addr);
    direct && (to.len() + cc.len()) as i64 <= super::index::MASS_MAIL
}

/// The latest five things with that address up to today, newest first: threads of mail they sent or received, and
/// events they attended (an event is known by its title: a weekly meeting is one).
fn person_threads(lib: &Librarian, vault: &Path, addr: &str) -> HashSet<String> {
    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
    let events = crate::assistant::util::files_like(&vault.join("Events"), "", ".md").into_iter().filter_map(|p| {
        let text = note::head(&p, 4000);
        let (front, _) = note::split(&text);
        let attended = front.to_lowercase().contains(&format!("<{addr}>"));
        let day: String = note::field(front, "start")?.chars().take(10).collect();
        (attended && day <= today).then(|| (day, format!("event:{}", note::field(front, "title").unwrap_or(""))))
    });
    let mut dated: Vec<(String, String)> = files_like_nested(&vault.join("Mail"), "", ".md")
        .into_iter()
        .filter_map(|p| {
            let f = p.to_string_lossy().to_string();
            let head = note::head(&p, 3000);
            if !with(&head, addr) {
                return None;
            }
            let when = format!("{} {}", note::field(&head, "date")?, note::field(&head, "time").unwrap_or(""));
            Some((when, lib.thread_of(&f)?))
        })
        .filter(|(when, _)| when.chars().take(10).collect::<String>() <= today)
        .chain(events)
        .collect();
    dated.sort_by(|a, b| b.0.cmp(&a.0));
    let mut out = Vec::new();
    for (_, t) in dated {
        if !out.contains(&t) {
            out.push(t);
        }
        if out.len() == 5 {
            break;
        }
    }
    out.into_iter().collect()
}

fn rank(lib: &Librarian, hits: &[serde_json::Value], targets: &HashSet<String>) -> Option<usize> {
    let key = |h: &serde_json::Value| match h["source"].as_str() {
        Some("event") => h["title"].as_str().map(|t| format!("event:{t}")),
        _ => h["ref"].as_str().and_then(|r| lib.thread_of(r)),
    };
    hits.iter().position(|h| key(h).is_some_and(|t| targets.contains(&t))).map(|i| i + 1)
}

/// Measure every case against the index, with the user's words and (if a model is given) the words it picks.
pub fn run(
    lib: &mut Librarian,
    model: Option<&dyn Model>,
    vault: &Path,
    cases: &[Case],
    k: usize,
) -> anyhow::Result<Vec<Result1>> {
    let mut out = Vec::new();
    for c in cases {
        let addr = c.person.as_ref().map(|a| a.trim().to_lowercase());
        let targets: HashSet<String> = match (c.kind, &c.thread, &addr) {
            (Kind::Person, _, Some(a)) => person_threads(lib, vault, a),
            (_, Some(t), _) => [t.clone()].into(),
            _ => HashSet::new(),
        };
        let scored = c.kind != Kind::Pending;
        let hits = lib.search(&c.q, k, None)?;
        let raw_rank = if scored { rank(lib, &hits, &targets) } else { None };
        let person_share = addr.as_ref().map(|a| {
            let with = hits.iter().filter(|h| h["ref"].as_str().is_some_and(|r| involves(r, a))).count();
            (with, hits.len())
        });
        let (model_words, model_rank) = match model {
            None => (None, None),
            Some(m) => match recall::pick(m, &c.q, None) {
                None => (Some(Vec::new()), None), // the model chose not to look anything up
                Some(p) => {
                    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                    // as the chat does today; asking back which one is not finding it
                    let hits = match recall::lookup(lib, &p, k, &today, &[])? {
                        recall::Named::Hits(h) => h,
                        recall::Named::Ask(_) => Vec::new(),
                    };
                    let r = if scored { rank(lib, &hits, &targets) } else { None };
                    let shown = p.people.iter().map(|n| format!("@{n}")).chain(p.words.iter().cloned()).collect();
                    (Some(shown), r)
                }
            },
        };
        out.push(Result1 { kind: c.kind, q: c.q.clone(), raw_rank, model_words, model_rank, person_share });
    }
    Ok(out)
}

/// Per kind (pending left out): (found, asked) with the user's words, and with the model's words.
#[derive(Debug, Default, PartialEq)]
pub struct Score {
    pub raw: (usize, usize),
    pub model: (usize, usize),
}

pub fn summary(results: &[Result1]) -> BTreeMap<Kind, Score> {
    let mut out: BTreeMap<Kind, Score> = BTreeMap::new();
    for r in results.iter().filter(|r| r.kind != Kind::Pending) {
        let s = out.entry(r.kind).or_default();
        s.raw = (s.raw.0 + r.raw_rank.is_some() as usize, s.raw.1 + 1);
        if r.model_words.is_some() {
            s.model = (s.model.0 + r.model_rank.is_some() as usize, s.model.1 + 1);
        }
    }
    out
}
