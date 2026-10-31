//! The graph (#45, decided in #42): who and what the records are about, and how they stand to each other, whatever
//! the user's work is.
//!
//! Nodes are `kind:key`:
//!   - `person:<address>`;
//!   - `org:<mail domain>` (public mail services are no organization; other names the records give it are aliases);
//!   - `case:<n>`, the matter (#8) a record is in;
//!   - for what only a name tells: `doc:`, `thing:`, `todo:`, `event:`, `person:`, `org:` with `~<name key>`.
//!
//! Edges keep the record they came from (`n`, `file`), the words that say it (`evidence`) and the day (`since`).
//! Two origins:
//!   - **rule**: who belongs where, from the mail headers; made again on every relink;
//!   - **model**: one ask per record, the relations of a fixed, work-neutral skeleton (role, state, sent, requested,
//!     todo, member, about, uses) with their labels in the record's own words. Each answer is checked:
//!     - the kinds an edge may join (a wrong way round is turned, the rest dropped);
//!     - the evidence must be in the record.
//!
//! The answers are kept by the record's hash under `<data>/cache/graph-<version>`: a rebuild puts them back without
//! asking again, and a record forgotten takes its edges with it.
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use rusqlite::{Connection, OptionalExtension, params};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::index::{Librarian, MASS_MAIL, name_key};
use crate::config::Paths;
use crate::llm::{Ask, Model};

/// bumped when the prompt or the reading of answers changes: kept answers of another version are not used
pub const VERSION: &str = "1";
/// what of a record the model reads
pub const MAX_RECORD: usize = 4000;
/// one record's ask
pub const TIMEOUT_S: u64 = 120;

/// Mail services anyone can use: an address there says nothing of where someone works.
const PUBLIC_MAIL: [&str; 18] = [
    "gmail.com",
    "googlemail.com",
    "naver.com",
    "daum.net",
    "hanmail.net",
    "kakao.com",
    "nate.com",
    "outlook.com",
    "hotmail.com",
    "live.com",
    "msn.com",
    "yahoo.com",
    "yahoo.co.kr",
    "icloud.com",
    "me.com",
    "proton.me",
    "protonmail.com",
    "hanmir.com",
];

pub const SYSTEM: &str = r#"너는 업무 기록에서 지식그래프를 뽑는다. 업무의 종류(영업·채용·개발·연구·행정 등)는 정해져 있지 않다. 아래 기록 하나에서 관계를 뽑는다.
개체 타입: person(사람), org(회사·기관·부서), case(일·건·프로젝트·과제·딜·채용·이슈), doc(문서·파일), todo(누가 해야 할 일), event(회의·방문·마감), thing(제품·시스템·장소 등 그 밖의 대상).
관계(src → dst), 이 여덟 가지만:
- role: person|org → case. label 에 그 건에서 맡은 역할을 기록의 말로(예: 고객, 담당, 공급, 지원자, 면접관, 리뷰어). 메일의 수신자·발신자·참조나 직급·직책(이사, 책임, 팀장)은 역할이 아니다.
- state: case → 날짜. label 에 그 건의 단계·상태를 기록의 말로 짧게(예: 견적 발송, 계약, 서류 접수, 면접, 배포, 보류).
- sent / requested: person|org → doc|todo. label 에 무엇인지 짧게.
- todo: person → todo (해야 할 사람). label 에 기한(있으면).
- member: person → org.
- about: doc|event|todo|thing → case.
- uses: org|case → thing.
규칙: 기록에 적힌 사실만. 광고·뉴스레터·시스템 알림이면 빈 목록. 서명·주소·전화·면책 문구는 관계로 만들지 않는다. 중요한 것부터 15개까지.
이름은 기록에 쓰인 대로, 타입을 뒤에 붙인다("이삭엔지니어링:org", 사람은 "이름 <주소>:person").
evidence: 기록에서 그대로 옮긴 40자 이내 구절. since: 그 사실의 날짜(없으면 기록 날짜).
JSON 만, 짧게: {"r":[["src:type","rel","dst:type","label","since","evidence"]]}"#;

/// The kinds an edge may join: (sources, targets; None for a day).
fn allowed(rel: &str) -> Option<(&'static [&'static str], Option<&'static [&'static str]>)> {
    Some(match rel {
        "role" => (&["person", "org"], Some(&["case"])),
        "state" => (&["case"], None),
        "sent" | "requested" => (&["person", "org"], Some(&["doc", "todo"])),
        "todo" => (&["person"], Some(&["todo"])),
        "member" => (&["person"], Some(&["org"])),
        "about" => (&["doc", "event", "todo", "thing"], Some(&["case"])),
        "uses" => (&["org", "case"], Some(&["thing"])),
        _ => return None,
    })
}

static DAY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap());
static ADDR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<([^<>\s]+@[^<>\s]+)>").unwrap());

/// A node of the graph.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Node {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub aliases: Vec<String>,
}

/// An edge, with where it was said.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Edge {
    pub src: String,
    pub rel: String,
    pub dst: String,
    pub label: String,
    pub since: String,
    /// the record's file
    pub doc: String,
    pub evidence: String,
    /// "rule" or "model"
    pub origin: String,
}

pub fn cache_dir(paths: &Paths) -> PathBuf {
    paths.data.join("cache").join(format!("graph-{VERSION}"))
}

fn domain(addr: &str) -> Option<&str> {
    addr.rsplit_once('@').map(|(_, d)| d).filter(|d| !d.is_empty() && !PUBLIC_MAIL.contains(d))
}

fn mine(db: &Connection) -> rusqlite::Result<HashSet<String>> {
    let mut st = db.prepare(
        "select distinct p.addr from people p join docs d on d.n = p.n where d.kind = '보낸 메일' and p.role = 'from'",
    )?;
    st.query_map([], |r| r.get(0))?.collect()
}

fn put_node(db: &Connection, id: &str, kind: &str, name: &str) -> rusqlite::Result<()> {
    db.execute(
        "insert into nodes(id, kind, name, aliases) values (?, ?, ?, '[]')
         on conflict(id) do update set name = case when nodes.name = '' then excluded.name else nodes.name end",
        params![id, kind, name],
    )?;
    Ok(())
}

fn add_alias(db: &Connection, id: &str, alias: &str) -> rusqlite::Result<()> {
    let Some(text) =
        db.query_row("select aliases from nodes where id = ?", [id], |r| r.get::<_, String>(0)).optional()?
    else {
        return Ok(());
    };
    let mut list: Vec<String> = serde_json::from_str(&text).unwrap_or_default();
    let a = alias.trim().to_string();
    if a.is_empty() || list.contains(&a) || list.len() >= 20 {
        return Ok(());
    }
    list.push(a);
    db.execute("update nodes set aliases = ? where id = ?", params![json!(list).to_string(), id])?;
    Ok(())
}

/// The rules' part, made again: people by address and the organizations of their mail domains, who belongs where
/// (since the first record that says so). The user is not one of the people.
pub fn rules(db: &Connection) -> rusqlite::Result<()> {
    db.execute("delete from edges where origin = 'rule'", [])?;
    let me = mine(db)?;
    let rows: Vec<(String, String, String, i64, String)> = {
        let mut st = db.prepare(&format!(
            "with mass as (select n from people where role in ('to', 'cc') group by n having count(*) > {MASS_MAIL})
             select g.addr, g.name, g.first, g.n, (select file from docs where n = g.n) from (
               select p.addr, p.name, min(d.day) as first, min(d.n) as n, count(*) as c
               from people p join docs d on d.n = p.n where p.n not in (select n from mass)
               group by p.addr, p.name) g
             order by g.addr, g.c desc"
        ))?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?
            .collect::<rusqlite::Result<_>>()?
    };
    let mut seen = HashSet::new();
    for (addr, name, first, n, file) in rows {
        if me.contains(&addr) || !seen.insert(addr.clone()) {
            continue; // the most used name of an address first
        }
        let person = format!("person:{addr}");
        put_node(db, &person, "person", &name)?;
        if let Some(d) = domain(&addr) {
            let org = format!("org:{d}");
            put_node(db, &org, "org", d)?;
            db.execute(
                "insert into edges(src, rel, dst, label, since, until, n, file, evidence, origin)
                 values (?, 'member', ?, '', ?, null, ?, ?, ?, 'rule')",
                params![person, org, first, n, file, addr],
            )?;
        }
    }
    // the organizations the address book gives a company name
    let book: Vec<(String, String)> = {
        let mut st = db.prepare("select addr, org from contacts where org != ''")?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
    };
    for (addr, org) in book {
        if let Some(d) = domain(&addr) {
            add_alias(db, &format!("org:{d}"), &org)?;
        }
    }
    Ok(())
}

/// The text of a record as the model reads it: its title, who and when, and its body.
fn record_text(title: &str, extra: &str, text: &str) -> String {
    let s = format!("subject: {title}\n{extra}\n\n{text}");
    s.chars().take(MAX_RECORD).collect()
}

fn sha(text: &str) -> String {
    hex::encode(Sha256::digest(format!("{VERSION}\n{SYSTEM}\n{text}").as_bytes()))
}

fn cache_file(dir: &Path, sha: &str) -> PathBuf {
    dir.join(&sha[..2]).join(format!("{sha}.json"))
}

/// A record the model reads: (n, file, title, unused, text).
type Candidate = (i64, String, String, String, String);

/// The records the model reads: mail, not mass mail.
fn candidates(db: &Connection) -> rusqlite::Result<Vec<Candidate>> {
    let mut st = db.prepare(&format!(
        "with mass as (select n from people where role in ('to', 'cc') group by n having count(*) > {MASS_MAIL})
         select d.n, d.file, d.title, '', d.text from docs d
         where d.source = 'mail' and d.n not in (select n from mass)
           and d.file not in (select file from graph_seen)
         order by d.day desc, d.n desc"
    ))?;
    st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)))?.collect()
}

/// The headers of a record as indexed (who, to whom, when), for the model.
fn heads(db: &Connection, n: i64) -> rusqlite::Result<String> {
    let mut st = db.prepare("select role, name, addr from people where n = ?")?;
    let lines: Vec<String> = st
        .query_map([n], |r| {
            Ok(format!("{}: {} <{}>", r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let day: String = db.query_row("select day from docs where n = ?", [n], |r| r.get(0))?;
    Ok(format!("date: {day}\n{}", lines.join("\n")))
}

/// Put back the kept answers of the records that have one and are not in the graph yet (after a rebuild, or a
/// record read again).
pub fn restore(db: &Connection, dir: &Path) -> anyhow::Result<usize> {
    let mut done = 0;
    for (n, file, title, _, text) in candidates(db)? {
        let full = record_text(&title, &heads(db, n)?, &text);
        let key = sha(&full);
        let Ok(answer) = std::fs::read_to_string(cache_file(dir, &key)) else { continue };
        apply(db, n, &file, &full, &answer)?;
        db.execute("insert into graph_seen(file, n, sha) values (?, ?, ?)", params![file, n, key])?;
        done += 1;
    }
    Ok(done)
}

/// One run of asking the model.
#[derive(Debug)]
pub struct Run {
    /// records asked about at most in this run
    pub limit: usize,
    /// asks at once
    pub workers: usize,
}

/// What a run did.
#[derive(Debug, Default, PartialEq)]
pub struct Done {
    pub asked: usize,
    pub failed: usize,
    pub edges: usize,
    /// records still waiting
    pub left: usize,
}

/// Ask the model about the records not in the graph yet (newest first), keep each answer and put its edges in.
/// A failed ask is tried again on a later run.
pub fn extract(lib: &mut Librarian, model: &dyn Model, run: &Run) -> anyhow::Result<Done> {
    let dir = cache_dir(&lib.paths);
    let todo: Vec<(i64, String, String)> = {
        let all = candidates(&lib.db)?;
        let mut out = Vec::new();
        for (n, file, title, _, text) in all {
            out.push((n, file, record_text(&title, &heads(&lib.db, n)?, &text)));
        }
        out
    };
    let left = todo.len().saturating_sub(run.limit);
    let todo: Vec<_> = todo.into_iter().take(run.limit).collect();
    // ask (outside the index's lock), a few at once
    let answers: Vec<(usize, Option<String>)> = {
        let next = std::sync::atomic::AtomicUsize::new(0);
        let out = std::sync::Mutex::new(Vec::new());
        std::thread::scope(|s| {
            for _ in 0..run.workers.max(1) {
                s.spawn(|| {
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let Some((_, _, text)) = todo.get(i) else { break };
                        let key = sha(text);
                        let kept = std::fs::read_to_string(cache_file(&dir, &key)).ok();
                        let got = kept.or_else(|| ask(model, text).ok());
                        out.lock().unwrap_or_else(|e| e.into_inner()).push((i, got));
                    }
                });
            }
        });
        out.into_inner().unwrap_or_else(|e| e.into_inner())
    };
    let mut done = Done { asked: todo.len(), left, ..Done::default() };
    lib.in_write(|me| {
        for (i, got) in &answers {
            let (n, file, text) = &todo[*i];
            let Some(answer) = got else {
                done.failed += 1;
                continue;
            };
            let key = sha(text);
            let path = cache_file(&dir, &key);
            if !path.exists() {
                crate::fsutil::ensure_private_dir(path.parent().unwrap_or(&dir))?;
                crate::fsutil::write_private_atomic(&path, answer.as_bytes())?;
            }
            // the record may have gone or changed while the model was asked
            let still: Option<i64> = me
                .db
                .query_row("select n from docs where n = ? and file = ?", params![n, file], |r| r.get(0))
                .optional()?;
            if still.is_none() {
                continue;
            }
            done.edges += apply(&me.db, *n, file, text, answer)?;
            me.db.execute("insert into graph_seen(file, n, sha) values (?, ?, ?)", params![file, n, key])?;
        }
        anyhow::Ok(())
    })?;
    Ok(done)
}

fn ask(model: &dyn Model, text: &str) -> anyhow::Result<String> {
    let messages = [json!({"role": "system", "content": SYSTEM}), json!({"role": "user", "content": text})];
    let answer = model.complete(&messages, Ask::new(0.0, 2000, true, TIMEOUT_S))?;
    crate::llm::parse_json(&answer)?; // only an answer that reads is kept
    Ok(answer)
}

/// Words that say how a mail went or what rank someone holds, not what they do in the matter.
const NOT_ROLES: [&str; 22] = [
    "수신",
    "수신자",
    "수신인",
    "발신",
    "발신자",
    "발신인",
    "참조",
    "참조자",
    "숨은참조",
    "cc",
    "bcc",
    "to",
    "from",
    "사원",
    "주임",
    "대리",
    "과장",
    "차장",
    "부장",
    "책임",
    "선임",
    "수석",
];

/// Is this a role (not empty, not only how the mail went or a rank)?
fn a_role(label: &str) -> bool {
    let k = name_key(label);
    !k.is_empty()
        && !NOT_ROLES.contains(&k.as_str())
        && !["이사", "상무", "전무", "대표", "팀장", "실장"].contains(&k.as_str())
}

/// `name:kind` -> (name, kind).
fn split(s: &str) -> (String, String) {
    match s.rsplit_once(':') {
        Some((name, kind)) if !kind.is_empty() && kind.chars().all(|c| c.is_ascii_lowercase()) => {
            (name.trim().to_string(), kind.to_string())
        }
        _ => (s.trim().to_string(), String::new()),
    }
}

fn squash(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Put the edges of one answer in. -> how many
fn apply(db: &Connection, n: i64, file: &str, record: &str, answer: &str) -> anyhow::Result<usize> {
    let Ok(v) = crate::llm::parse_json(answer) else { return Ok(0) };
    let Some(rows) = v.get("r").and_then(Value::as_array) else { return Ok(0) };
    let record_flat = squash(&super::index::fold(record));
    let day: String = db.query_row("select day from docs where n = ?", [n], |r| r.get(0))?;
    let case: String = db
        .query_row("select case_key from docs where n = ?", [n], |r| r.get::<_, Option<String>>(0))?
        .unwrap_or_else(|| format!("case:doc{n}"));
    // the people of this record, to know a person the answer names without an address
    let people: Vec<(String, String)> = {
        let mut st = db.prepare("select name, addr from people where n = ?")?;
        st.query_map([n], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
    };
    let me = mine(db)?;
    let mut added = 0;
    for row in rows.iter().take(30) {
        let Some(f) = row.as_array().filter(|f| f.len() >= 6) else { continue };
        let s = |i: usize| super::super::assistant::util::pystr(f.get(i)).trim().to_string();
        let (rel, label, since, evidence) = (s(1), s(3), s(4), s(5));
        let Some((from, to)) = allowed(&rel) else { continue };
        let ev = squash(&super::index::fold(&evidence));
        if ev.chars().count() < 2 || !record_flat.contains(&ev) {
            continue; // words the record does not hold
        }
        let (mut a, mut b) = (split(&s(0)), split(&s(2)));
        let fits = |x: &(String, String), kinds: &[&str]| kinds.contains(&x.1.as_str());
        let ok = |a: &(String, String), b: &(String, String)| fits(a, from) && to.is_none_or(|t| fits(b, t));
        if !ok(&a, &b) {
            if to.is_some() && ok(&b, &a) {
                std::mem::swap(&mut a, &mut b); // said the wrong way round
            } else {
                continue;
            }
        }
        let since = DAY.find(&since).map(|m| m.as_str().to_string()).unwrap_or_else(|| day.clone());
        let src = resolve(db, &a, &case, &people)?;
        let Some(src) = src else { continue };
        if me.iter().any(|m| src == format!("person:{m}")) && rel == "member" {
            continue;
        }
        let dst = if to.is_none() {
            // a state: the day it holds from
            String::new()
        } else {
            match resolve(db, &b, &case, &people)? {
                Some(d) => d,
                None => continue,
            }
        };
        let since = if to.is_none() { DAY.find(&b.0).map(|m| m.as_str().to_string()).unwrap_or(since) } else { since };
        if rel == "member" && src.starts_with("person:") && !src.contains('~') {
            // the rules know where an address belongs: the model's name for it is a name of that organization
            if let Some(org) = src.strip_prefix("person:").and_then(domain) {
                add_alias(db, &format!("org:{org}"), &b.0)?;
                continue;
            }
        }
        if rel == "role" && !a_role(&label) {
            continue;
        }
        let label: String = label.chars().take(60).collect();
        let evidence: String = evidence.chars().take(80).collect();
        db.execute(
            "insert into edges(src, rel, dst, label, since, until, n, file, evidence, origin)
             values (?, ?, ?, ?, ?, null, ?, ?, ?, 'model')",
            params![src, rel, dst, label, since, n, file, evidence],
        )?;
        added += 1;
    }
    Ok(added)
}

/// The node a name of the answer is, made if new: the record's matter, a person of the record, an organization by
/// a name it is known by, else a node of that name.
fn resolve(
    db: &Connection,
    x: &(String, String),
    case: &str,
    people: &[(String, String)],
) -> rusqlite::Result<Option<String>> {
    let (name, kind) = x;
    let key = name_key(name);
    if key.chars().count() < 2 && kind != "case" {
        return Ok(None);
    }
    let id = match kind.as_str() {
        "case" => {
            put_node(db, case, "case", name)?;
            add_alias(db, case, name)?;
            return Ok(Some(case.to_string()));
        }
        "person" => {
            let addr = ADDR.captures(name).map(|c| c[1].to_lowercase()).or_else(|| {
                let bare = ADDR.replace(name, "");
                let k = name_key(&bare);
                people.iter().find(|(n, _)| !k.is_empty() && name_key(n).contains(&k)).map(|(_, a)| a.clone())
            });
            match addr {
                Some(a) => format!("person:{a}"),
                None => format!("person:~{key}"),
            }
        }
        "org" => {
            let known: Option<String> = db
                .query_row(
                    "select id from nodes where kind = 'org' and (name = ?1 or instr(aliases, ?2) > 0) limit 1",
                    params![name, json!(name).to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            known.unwrap_or_else(|| format!("org:~{key}"))
        }
        "doc" | "todo" | "event" | "thing" => format!("{kind}:~{key}"),
        _ => return Ok(None),
    };
    let shown = ADDR.replace(name, "").trim().to_string();
    put_node(db, &id, kind, &shown)?;
    Ok(Some(id))
}

impl Librarian {
    pub fn node(&self, id: &str) -> Option<Node> {
        self.db
            .query_row("select id, kind, name, aliases from nodes where id = ?", [id], |r| {
                Ok(Node {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    name: r.get(2)?,
                    aliases: serde_json::from_str(&r.get::<_, String>(3)?).unwrap_or_default(),
                })
            })
            .optional()
            .ok()
            .flatten()
    }

    fn edges(&self, side: &str, id: &str, rel: Option<&str>) -> Vec<Edge> {
        let sql = format!(
            "select src, rel, dst, label, since, file, evidence, origin from edges where {side} = ?1 and (?2 is null or rel = ?2)
             order by since desc"
        );
        self.db
            .prepare(&sql)
            .and_then(|mut st| {
                st.query_map(params![id, rel], |r| {
                    Ok(Edge {
                        src: r.get(0)?,
                        rel: r.get(1)?,
                        dst: r.get(2)?,
                        label: r.get(3)?,
                        since: r.get(4)?,
                        doc: r.get(5)?,
                        evidence: r.get(6)?,
                        origin: r.get(7)?,
                    })
                })?
                .collect()
            })
            .unwrap_or_default()
    }

    pub fn edges_from(&self, id: &str, rel: Option<&str>) -> Vec<Edge> {
        self.edges("src", id, rel)
    }

    pub fn edges_to(&self, id: &str, rel: Option<&str>) -> Vec<Edge> {
        self.edges("dst", id, rel)
    }
}

/// How many records wait for the model, of how many it reads.
pub fn backlog(db: &Connection) -> rusqlite::Result<usize> {
    Ok(candidates(db)?.len())
}
