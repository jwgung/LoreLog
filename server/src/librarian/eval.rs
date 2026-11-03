//! Measuring the librarian (#7): a file of questions, each with the record it should find, searched the two ways the
//! assistant can (the user's words as they are, and the words the model picks), and the rank the record comes back
//! at. The file holds real names and subjects, so it lives outside the repository, in the vault (see
//! `life-librarian eval`).
//!
//! Kinds of question and what counts as found in the top `k`:
//!   status     "그 건 어떻게 됐지"        the thread it is about
//!   precedent  "전에 어떻게 했지"          the earlier thread it is about; with `earlier`, also whether the earlier
//!                                          matters like the one at hand (#95) hold every one of those threads
//!   person     "이 사람이랑 뭐 있었지"      any of the latest five things with that address up to today: threads of mail
//!                                          they sent or received (not copied, not mass mail) and events they attended
//!                                          (read from the notes, not from the index), and how many hits are with them
//!   graph      "A사랑 하는 건들 어디까지"  several matters (#44): found when every one of them is, at the rank of the last
//!   period     "이번 달에 뭐가 걸려 있지"   what hangs in a period (#61), asked on a fixed `day`: found when every one of the
//!                                          `items` (words of a line: a title) is in the section, at the line of the last
//!                                          (the model only; the user's words name no period)
//!   conflict   "기록끼리 다른 건"          records that say different things (#92): found when every one of them is shown
//!                                          (a hit's own mail, not just its matter: a matter is one hit), at the rank of the last
//!   pending    (the same, before #61)       not scored: kept to see what is asked
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
    /// a question about several matters at once (#44)
    Graph,
    /// what hangs in a period, asked on a fixed day (#61)
    Period,
    /// records that disagree: every one must be shown (#92)
    Conflict,
    Pending,
}

impl Kind {
    pub fn label(self) -> &'static str {
        match self {
            Kind::Status => "그 건 어떻게 됐지",
            Kind::Person => "이 사람이랑 뭐 있었지",
            Kind::Precedent => "전에 어떻게 했지",
            Kind::Graph => "여러 건을 따라가는 질문",
            Kind::Period => "이번 달에 뭐가 걸려 있지",
            Kind::Conflict => "기록끼리 다른 건",
            Kind::Pending => "이번 달에 뭐가 걸려 있지 (채점 안 함)",
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
    /// precedent: the threads of the earlier matters with the same counterparty it should gather (#95), when known
    pub earlier: Option<Vec<String>>,
    /// person: their mail address
    pub person: Option<String>,
    /// graph: the threads of every matter it is about
    pub threads: Option<Vec<String>>,
    /// conflict: the mails that disagree (their message ids, as `mail_id`), two or more
    pub records: Option<Vec<String>>,
    /// period: the day it is asked on (`YYYY-MM-DD`)
    pub day: Option<String>,
    /// period: what its section must hold, a title (or words of its line) each
    pub items: Option<Vec<String>>,
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
            Kind::Graph if c.threads.as_ref().is_none_or(|t| t.is_empty()) => Some("threads"),
            Kind::Conflict if c.records.as_ref().is_none_or(|r| r.len() < 2) => Some("records"),
            Kind::Period
                if c.day.as_deref().is_none_or(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_err()) =>
            {
                Some("day")
            }
            Kind::Period if c.items.as_ref().is_none_or(|t| t.is_empty()) => Some("items"),
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
    /// the model's words made the assistant ask back which one is meant (#93): `model_rank` is then the place of the
    /// candidate that holds the record, not of a hit
    pub asked: bool,
    /// person: (hits with that person, hits) for the user's words
    pub person_share: Option<(usize, usize)>,
    /// precedent with `earlier`, the model's words: were all of them among the earlier matters gathered (#95)? Asked
    /// back: not (they come only after the answer).
    pub earlier_shown: Option<bool>,
    /// the model's words gathered earlier matters like the one at hand (#95): how many, and whether by the same
    /// counterparty (else by the situation)
    pub gathered: Option<(usize, bool)>,
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

/// Where the first hit that holds a target is: a hit is a matter, which holds the threads and events of it (#8).
fn rank(lib: &Librarian, hits: &[serde_json::Value], targets: &HashSet<String>) -> Option<usize> {
    let holds = |h: &serde_json::Value| {
        h["ref"].as_str().is_some_and(|r| !lib.case_threads(r).is_disjoint(targets))
            || (h["source"] == "event" && h["title"].as_str().is_some_and(|t| targets.contains(&format!("event:{t}"))))
    };
    hits.iter().position(holds).map(|i| i + 1)
}

/// Where every target is held by some hit: the rank of the last of them, or None when one is not in the hits.
fn rank_all(lib: &Librarian, hits: &[serde_json::Value], targets: &HashSet<String>) -> Option<usize> {
    let mut last = 0;
    for t in targets {
        let one: HashSet<String> = [t.clone()].into();
        last = last.max(rank(lib, hits, &one)?);
    }
    Some(last)
}

/// Where every record is shown: the rank of the last of them, or None when one is not shown. A matter (a thread, or mails
/// joined to it) is one hit with one of its mails and up to a few `others` (ADR 0003); a mail beyond those is not shown.
fn shown_all(lib: &Librarian, hits: &[serde_json::Value], records: &[String]) -> Option<usize> {
    let mut last = 0;
    for r in records {
        let file = lib.file_of_mail(r)?;
        // a mail folded behind a hit of its matter is shown with it, in that hit's `others`
        // (a copy of it, the same day and text, shown in its place says the same)
        let is = |r: &serde_json::Value| r.as_str().is_some_and(|r| r == file || lib.same_mail(r, &file));
        let shows = |h: &serde_json::Value| {
            is(&h["ref"]) || h["others"].as_array().is_some_and(|o| o.iter().any(|o| is(&o["ref"])))
        };
        last = last.max(hits.iter().position(shows)? + 1);
    }
    Some(last)
}

/// The words the model picked as shown: `@person`, `#organization`, `^thing`, `~period`, `=전에` for a question about
/// how it was done before (#95), then the words.
fn shown(p: &recall::Picked) -> Vec<String> {
    p.people
        .iter()
        .map(|n| format!("@{n}"))
        .chain(p.orgs.iter().map(|o| format!("#{o}")))
        .chain(p.things.iter().map(|t| format!("^{t}")))
        .chain(p.period.map(|x| format!("~{}", x.label())))
        .chain(p.precedent.then(|| "=전에".to_string()))
        .chain(p.words.iter().cloned())
        .collect()
}

/// Where the matter meant is among the candidates asked back (1-based), or None when the user would not find it there:
/// people of one name are told apart by address, matters by the threads they hold. Several matters (graph): every one.
fn in_candidates(
    lib: &Librarian,
    asked: &recall::Asked,
    kind: Kind,
    targets: &HashSet<String>,
    addr: Option<&str>,
) -> Option<usize> {
    let holds: Vec<HashSet<String>> = match asked {
        recall::Asked::People(cs) => {
            return cs.iter().position(|c| addr.is_some_and(|a| c.addr.to_lowercase() == a)).map(|i| i + 1);
        }
        a @ (recall::Asked::Matters(_) | recall::Asked::Now(_)) => {
            a.matters().iter().map(|m| lib.key_threads(&m.case)).collect()
        }
    };
    let place = |t: &String| holds.iter().position(|h| h.contains(t)).map(|i| i + 1);
    match kind {
        Kind::Graph => targets.iter().map(place).collect::<Option<Vec<_>>>()?.into_iter().max(),
        _ => targets.iter().filter_map(place).min(),
    }
}

/// The line (1-based) of the last item expected in a period's section, or None when one is not in it.
fn in_section(section: &str, items: &[String]) -> Option<usize> {
    let lines: Vec<&str> = section.lines().collect();
    let mut last = 0;
    for item in items {
        last = last.max(lines.iter().position(|l| l.contains(item.as_str()))? + 1);
    }
    Some(last)
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
    for (i, c) in cases.iter().enumerate() {
        // a record the index does not hold is a broken case, not a miss: it would pass for "not shown"
        if c.kind == Kind::Conflict && c.records.iter().flatten().any(|r| lib.file_of_mail(r).is_none()) {
            anyhow::bail!("case {} (Conflict): a record is not in the index", i + 1);
        }
        let addr = c.person.as_ref().map(|a| a.trim().to_lowercase());
        let targets: HashSet<String> = match (c.kind, &c.thread, &addr) {
            (Kind::Person, _, Some(a)) => person_threads(lib, vault, a),
            (Kind::Graph, _, _) => c.threads.iter().flatten().cloned().collect(),
            (_, Some(t), _) => [t.clone()].into(),
            _ => HashSet::new(),
        };
        let hits = lib.search(&c.q, k, None)?;
        let score = |lib: &Librarian, hits: &[serde_json::Value]| match c.kind {
            Kind::Pending | Kind::Period => None,
            Kind::Graph => rank_all(lib, hits, &targets),
            Kind::Conflict => shown_all(lib, hits, c.records.as_deref().unwrap_or(&[])),
            _ => rank(lib, hits, &targets),
        };
        let raw_rank = score(lib, &hits);
        let person_share = addr.as_ref().map(|a| {
            let with = hits.iter().filter(|h| h["ref"].as_str().is_some_and(|r| involves(r, a))).count();
            (with, hits.len())
        });
        let mut asked = false;
        let mut earlier_shown = None;
        let mut gathered = None;
        let earlier: HashSet<String> = c.earlier.iter().flatten().cloned().collect();
        let (model_words, model_rank) = match model {
            None => (None, None),
            Some(m) => match recall::pick(m, &c.q, None) {
                None => (Some(Vec::new()), None), // the model chose not to look anything up
                Some(p) if c.kind == Kind::Period => {
                    // what hangs on the day the case is asked on, as the chat gathers it
                    let day = c.day.clone().unwrap_or_default();
                    let r = match p.period {
                        Some(_) => {
                            let section = recall::period_section(lib, &p, &day)?.section;
                            in_section(&section, c.items.as_deref().unwrap_or(&[]))
                        }
                        None => None, // no period named: not answered as one
                    };
                    (Some(shown(&p)), r)
                }
                // the chat would answer it with what hangs in a period, not with the records it is about
                Some(p) if p.period.is_some() => (Some(shown(&p)), None),
                Some(p) => {
                    let today = chrono::Local::now().format("%Y-%m-%d").to_string();
                    // as the chat does today; asking back is scored by the candidates it lists (#93)
                    let r = match recall::lookup(lib, &p, k, &today, &[])? {
                        recall::Named::Hits(h) => {
                            let like: Vec<&str> = h.iter().filter_map(|h| h.get("like_kind")?.as_str()).collect();
                            if let Some(first) = like.first() {
                                gathered = Some((like.len(), *first == super::precedent::BY_COUNTERPARTY));
                            }
                            if !earlier.is_empty() {
                                let like: Vec<serde_json::Value> =
                                    h.iter().filter(|h| h.get("like").is_some()).cloned().collect();
                                earlier_shown = Some(rank_all(lib, &like, &earlier).is_some());
                            }
                            score(lib, &h)
                        }
                        recall::Named::Ask(a) => {
                            earlier_shown = (!earlier.is_empty()).then_some(false);
                            asked = true;
                            in_candidates(lib, &a, c.kind, &targets, addr.as_deref())
                        }
                    };
                    (Some(shown(&p)), r)
                }
            },
        };
        out.push(Result1 {
            kind: c.kind,
            q: c.q.clone(),
            raw_rank,
            model_words,
            model_rank,
            person_share,
            asked,
            earlier_shown,
            gathered,
        });
    }
    Ok(out)
}

/// Per kind (pending left out; a period is measured with the model's pick only): (found, asked) with the user's words, and with the model's words.
/// `asked`: how many times the assistant asked back instead of answering (counted apart from found, #93).
#[derive(Debug, Default, PartialEq)]
pub struct Score {
    pub raw: (usize, usize),
    pub model: (usize, usize),
    pub asked: usize,
    /// precedent: (cases whose earlier matters were all gathered, cases with `earlier`), the model's words (#95)
    pub earlier: (usize, usize),
}

impl Score {
    /// How often the assistant asked back, out of the questions its words were picked for (`되물음 3/15`); None when
    /// no model was asked (nothing is asked back then). A specific question should not be asked back (#93).
    pub fn asked_label(&self) -> Option<String> {
        (self.model.1 > 0).then(|| format!("되물음 {}/{}", self.asked, self.model.1))
    }
}

pub fn summary(results: &[Result1]) -> BTreeMap<Kind, Score> {
    let mut out: BTreeMap<Kind, Score> = BTreeMap::new();
    for r in results.iter().filter(|r| r.kind != Kind::Pending) {
        let s = out.entry(r.kind).or_default();
        if r.kind != Kind::Period {
            s.raw = (s.raw.0 + r.raw_rank.is_some() as usize, s.raw.1 + 1);
        }
        if r.model_words.is_some() {
            s.model = (s.model.0 + r.model_rank.is_some() as usize, s.model.1 + 1);
            s.asked += r.asked as usize;
        }
        if let Some(e) = r.earlier_shown {
            s.earlier = (s.earlier.0 + e as usize, s.earlier.1 + 1);
        }
    }
    out
}
