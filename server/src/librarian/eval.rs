//! Measuring the librarian (#7): a file of questions, each with the record it should find, searched the two ways the
//! assistant can (the user's words as they are, and the words the model picks), and the rank the record comes back
//! at. The file holds real names and subjects, so it lives outside the repository, in the vault (see
//! `life-librarian eval`).
//!
//! Kinds of question and what counts as found in the top `k`:
//!   status     "그 건 어떻게 됐지"        the thread it is about
//!   precedent  "전에 어떻게 했지"          the earlier thread it is about
//!   person     "이 사람이랑 뭐 있었지"      any of the latest five threads with that address (and how many of the hits
//!                                          are with them)
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
struct File {
    #[serde(rename = "case", default)]
    cases: Vec<Case>,
}

/// The cases of a file. Err: not TOML, an unknown kind, or a case without what it should find.
pub fn load(path: &Path) -> anyhow::Result<Vec<Case>> {
    let f: File = toml::from_str(&std::fs::read_to_string(path)?)?;
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

/// The threads of the latest five mails with that address, newest first.
fn person_threads(lib: &Librarian, vault: &Path, addr: &str) -> HashSet<String> {
    let mut dated: Vec<(String, String)> = files_like_nested(&vault.join("Mail"), "", ".md")
        .into_iter()
        .filter_map(|p| {
            let f = p.to_string_lossy().to_string();
            if !involves(&f, addr) {
                return None;
            }
            let head = note::head(&p, 3000);
            let when = format!("{} {}", note::field(&head, "date")?, note::field(&head, "time").unwrap_or(""));
            Some((when, lib.thread_of(&f)?))
        })
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
    hits.iter()
        .position(|h| h["ref"].as_str().and_then(|r| lib.thread_of(r)).is_some_and(|t| targets.contains(&t)))
        .map(|i| i + 1)
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
            Some(m) => match recall::search_words(m, &c.q) {
                None => (Some(Vec::new()), None), // the model chose not to look anything up
                Some(words) => {
                    let hits = lib.search(&words.join(" "), k, None)?;
                    let r = if scored { rank(lib, &hits, &targets) } else { None };
                    (Some(words), r)
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
