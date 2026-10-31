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

/// The mail domains of the user's own addresses: their own organization, never the one a name stands for.
fn own_domains(db: &Connection) -> rusqlite::Result<HashSet<String>> {
    Ok(mine(db)?.iter().filter_map(|a| domain(a).map(str::to_string)).collect())
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

// ---------- asking the graph (#46) ----------

/// A name as organizations are compared: [`name_key`], without what says it is a company ((주), 주식회사, Inc …), and
/// with the vowels people spell either way made one (ㅐ/ㅔ, ㅒ/ㅖ, ㅙ/ㅞ/ㅚ): `(주)설악이앤씨` and `설악이엔씨` alike.
pub fn org_key(name: &str) -> String {
    let mut k = name_key(name);
    for mark in [
        "(주)",
        "㈜",
        "주식회사",
        "(유)",
        "유한회사",
        "co.,ltd",
        "co.ltd",
        "co,ltd",
        "ltd",
        "inc",
        "corp",
        "corporation",
        "company",
    ] {
        k = k.replace(mark, "");
    }
    k.chars()
        .map(|c| {
            let o = c as u32;
            if !(0xAC00..=0xD7A3).contains(&o) {
                return c;
            }
            let s = o - 0xAC00;
            let (lead, vowel, tail) = (s / 588, (s % 588) / 28, s % 28);
            let vowel = match vowel {
                1 => 5,        // ㅐ -> ㅔ
                3 => 7,        // ㅒ -> ㅖ
                10 | 11 => 15, // ㅙ, ㅚ -> ㅞ
                v => v,
            };
            char::from_u32(0xAC00 + lead * 588 + vowel * 28 + tail).unwrap_or(c)
        })
        .collect()
}

/// Short names the model guessed for organizations (org id -> names). Kept apart from the aliases: a guess is no
/// fact of the records, only a way to understand the user.
fn short_names(paths: &Paths) -> std::collections::HashMap<String, Vec<String>> {
    std::fs::read_to_string(cache_dir(paths).join("orgs-short.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

/// Ask the model what the organizations the records know by full name are also called (initials, the name without
/// its kind, the English name), a batch of those not asked yet with the most records. -> organizations asked about
pub fn abbreviate_orgs(lib: &Librarian, model: &dyn Model) -> anyhow::Result<usize> {
    let mut known = short_names(&lib.paths);
    let rows: Vec<(String, String, String)> = {
        let mut st = lib.db.prepare(
            "select n.id, n.name, n.aliases from nodes n where n.kind = 'org' and n.id not like 'org:~%'
             order by (select count(*) from edges e where e.dst = n.id and e.rel = 'member') desc",
        )?;
        st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?
    };
    let todo: Vec<(String, Vec<String>)> = rows
        .into_iter()
        .filter(|(id, ..)| !known.contains_key(id))
        .map(|(id, name, a)| {
            let names: Vec<String> =
                std::iter::once(name).chain(serde_json::from_str::<Vec<String>>(&a).unwrap_or_default()).collect();
            (id, names)
        })
        .take(40)
        .collect();
    if todo.is_empty() {
        return Ok(0);
    }
    let system = "회사·기관 목록을 보고, 사람들이 말로 부를 때 쓰는 다른 이름을 적는다. 두 가지를 적는다: (1) 줄여 부르는 이름 — 영문 약자, 앞뒤를 뗀 이름. (2) 영어 이름이나 도메인만 있으면 그것을 한글로 읽은 이름. 목록에 이미 있는 이름은 빼고 새 이름만. 모르면 빈 목록.\n예) 입력:\norg:ksa.or.kr: ksa.or.kr / 한국표준협회\norg:softwareag.com: softwareag.com / Software AG\norg:kakaocorp.com: kakaocorp.com\n출력: {\"a\": {\"org:ksa.or.kr\": [\"KSA\", \"표준협회\"], \"org:softwareag.com\": [\"소프트웨어에이지\", \"SAG\"], \"org:kakaocorp.com\": [\"카카오\"]}}\nJSON 만.";
    let user: String = todo.iter().map(|(id, names)| format!("{id}: {}\n", names.join(" / "))).collect();
    let answer = model.complete(
        &[json!({"role": "system", "content": system}), json!({"role": "user", "content": user})],
        Ask::new(0.0, 3000, true, 300),
    )?;
    let v = crate::llm::parse_json(&answer)?;
    for (id, _) in &todo {
        // the answer's map may be under "a" or not, keyed with or without the "org:"
        let map = if v.get("a").is_some_and(Value::is_object) { &v["a"] } else { &v };
        let bare = id.trim_start_matches("org:");
        let names: Vec<String> = [id.as_str(), bare]
            .iter()
            .find_map(|k| map.get(*k).and_then(Value::as_array))
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(|s| s.trim().chars().take(40).collect::<String>())
                    .filter(|s| s.chars().count() >= 2)
                    .take(6)
                    .collect()
            })
            .unwrap_or_default();
        known.insert(id.clone(), names); // asked: an empty list is not asked again
    }
    crate::fsutil::ensure_private_dir(&cache_dir(&lib.paths))?;
    crate::fsutil::write_private_atomic(
        &cache_dir(&lib.paths).join("orgs-short.json"),
        json!(known).to_string().as_bytes(),
    )?;
    Ok(todo.len())
}

/// What a question asks the graph: who and what it is about, and how the matters it wants stand.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ask1 {
    pub people: Vec<String>,
    /// the other words: one that is exactly an organization's name is asked about as it
    pub words: Vec<String>,
    pub orgs: Vec<String>,
    pub things: Vec<String>,
    /// roles someone has in the matters (the record's words, or a group of them)
    pub roles: Vec<String>,
    /// how the matters stand (their last state)
    pub states: Vec<String>,
    /// what did not follow that state
    pub absent: Vec<String>,
    /// requests the user has not answered
    pub open_requests: bool,
}

impl Ask1 {
    pub fn is_empty(&self) -> bool {
        self.orgs.is_empty()
            && self.things.is_empty()
            && self.roles.is_empty()
            && self.states.is_empty()
            && self.absent.is_empty()
            && !self.open_requests
    }
}

fn label_fits(label: &str, words: &[String], groups: &std::collections::HashMap<String, String>) -> bool {
    let l = name_key(label);
    let g = groups.get(label).map(|g| name_key(g)).unwrap_or_default();
    words
        .iter()
        .map(|w| name_key(w))
        .filter(|w| !w.is_empty())
        .any(|w| l.contains(&w) || (!g.is_empty() && g.contains(&w)))
}

/// The groups of the labels the model gave (label -> group), kept by [`group_labels`].
fn groups(paths: &Paths) -> std::collections::HashMap<String, String> {
    std::fs::read_to_string(cache_dir(paths).join("labels.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

impl Librarian {
    /// The organizations a name is: by name, alias or the address book; else the mail domains of whoever sends the
    /// mails that name it (an organization named only in its subjects, `[한국표준협회]`).
    fn orgs_named(&self, name: &str) -> anyhow::Result<Vec<String>> {
        let key = org_key(name);
        if key.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let short = short_names(&self.paths);
        let mut out: Vec<String> = {
            let mut st = self.db.prepare("select id, name, aliases from nodes where kind = 'org'")?;
            st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
                .filter_map(Result::ok)
                .filter(|(id, n, a)| {
                    let names: Vec<String> = std::iter::once(n.clone())
                        .chain(serde_json::from_str::<Vec<String>>(a).unwrap_or_default())
                        .chain(short.get(id).cloned().unwrap_or_default())
                        .collect();
                    // the domain's first part is a name too (ksa of ksa.or.kr)
                    let label = id.strip_prefix("org:").and_then(|d| d.split('.').next()).unwrap_or("");
                    org_key(label) == key
                        || names.iter().any(|x| {
                            let k = org_key(x);
                            k == key
                                || k.contains(&key) && key.chars().count() >= 3
                                || (k.chars().count() >= 3 && key.contains(&k))
                        })
                })
                .map(|(id, _, _)| id)
                .collect()
        };
        // a name-only organization is the one whose people send the records it is named in
        let named_only: Vec<String> = out.iter().filter(|id| id.starts_with("org:~")).cloned().collect();
        for id in named_only {
            if let Some(d) = self.domain_of_named(&id)? {
                out.push(format!("org:{d}"));
            }
        }
        out.sort();
        out.dedup();
        // no organization with a domain yet: the domain of whoever sends the records that name it, by this name and
        // by the spellings the records use for it
        if !out.iter().any(|id| !id.starts_with("org:~")) {
            let spellings: Vec<String> = std::iter::once(name.to_string())
                .chain(out.iter().filter_map(|id| self.node(id).map(|n| n.name)))
                .collect();
            let expr = spellings
                .iter()
                .map(|n| super::index::to_query(n))
                .filter(|e| !e.is_empty())
                .collect::<Vec<_>>()
                .join(" OR ");
            if !expr.is_empty() {
                let me = mine(&self.db)?;
                let own = own_domains(&self.db)?;
                let mut count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
                let mut st = self.db.prepare(
                    "select p.addr from fts join docs d on d.n = fts.rowid join people p on p.n = d.n
                     where fts match ? and d.source = 'mail' and p.role = 'from' limit 200",
                )?;
                for a in st.query_map([&expr], |r| r.get::<_, String>(0))?.flatten() {
                    if !me.contains(&a)
                        && let Some(d) = domain(&a)
                        && !own.contains(d)
                    {
                        *count.entry(d.to_string()).or_default() += 1;
                    }
                }
                let total: usize = count.values().sum();
                let mut by: Vec<(String, usize)> = count.into_iter().collect();
                by.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
                // the domain most of them come from (not one sender among many)
                if let Some((d, n)) = by.first()
                    && n * 2 >= total
                {
                    out.push(format!("org:{d}"));
                }
            }
        }
        Ok(out)
    }

    /// The mail domain most of the records naming a name-only organization come from, if most do.
    fn domain_of_named(&self, id: &str) -> anyhow::Result<Option<String>> {
        let me = mine(&self.db)?;
        let own = own_domains(&self.db)?;
        let mut st = self.db.prepare(
            "select p.addr from edges e join people p on p.n = e.n where (e.src = ?1 or e.dst = ?1) and p.role = 'from'",
        )?;
        let mut count: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for a in st.query_map([id], |r| r.get::<_, String>(0))?.flatten() {
            if !me.contains(&a)
                && let Some(d) = domain(&a)
                && !own.contains(d)
            {
                *count.entry(d.to_string()).or_default() += 1;
            }
        }
        let total: usize = count.values().sum();
        Ok(count.into_iter().max_by_key(|(_, n)| *n).filter(|(_, n)| n * 2 > total).map(|(d, _)| d))
    }

    /// The nodes of other kinds a name is (people, things, matters by the name the model gave them).
    fn nodes_named(&self, name: &str, kinds: &[&str]) -> anyhow::Result<Vec<String>> {
        let key = name_key(name);
        if key.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let mut st = self.db.prepare("select id, kind, name, aliases from nodes")?;
        let rows: Vec<(String, String, String, String)> =
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(rows
            .into_iter()
            .filter(|(_, k, n, a)| {
                kinds.contains(&k.as_str())
                    && std::iter::once(n.clone())
                        .chain(serde_json::from_str::<Vec<String>>(a).unwrap_or_default())
                        .any(|x| name_key(&x).contains(&key))
            })
            .map(|(id, ..)| id)
            .collect())
    }

    /// The matters with any of these nodes: the records of an organization's people (by mail domain) and of a person,
    /// and the matters an edge joins them to.
    fn cases_with(&self, ids: &[String]) -> anyhow::Result<HashSet<String>> {
        let mut out = HashSet::new();
        for id in ids {
            let mut add = |sql: &str, arg: &str| -> anyhow::Result<()> {
                let mut st = self.db.prepare(sql)?;
                for c in st.query_map([arg], |r| r.get::<_, Option<String>>(0))?.flatten().flatten() {
                    out.insert(c);
                }
                Ok(())
            };
            let mass =
                format!("(select n from people where role in ('to', 'cc') group by n having count(*) > {MASS_MAIL})");
            if let Some(d) = id.strip_prefix("org:").filter(|d| !d.starts_with('~')) {
                add(
                    &format!(
                        "select distinct d.case_key from people p join docs d on d.n = p.n
                         where p.addr like '%@' || ? and d.n not in {mass}"
                    ),
                    d,
                )?;
            }
            if let Some(a) = id.strip_prefix("person:").filter(|a| !a.starts_with('~')) {
                add(
                    &format!(
                        "select distinct d.case_key from people p join docs d on d.n = p.n where p.addr = ? and d.n not in {mass}"
                    ),
                    a,
                )?;
            }
            add("select ?1 where ?1 like 'case:%'", id)?;
            add("select dst from edges where src = ? and dst like 'case:%'", id)?;
            add("select src from edges where dst = ? and src like 'case:%'", id)?;
            // a name-only organization: the matters of its people
            add(
                "select e2.dst from edges e1 join edges e2 on e2.src = e1.src and e2.dst like 'case:%' where e1.dst = ? and e1.rel = 'member'",
                id,
            )?;
        }
        Ok(out)
    }

    /// The last state of a matter up to `day`: (label, since).
    fn last_state(&self, case: &str, day: &str) -> Option<(String, String)> {
        self.db
            .query_row(
                "select label, since from edges where src = ? and rel = 'state' and since <= ? order by since desc limit 1",
                params![case, day],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()
            .ok()
            .flatten()
    }

    /// The organization a name is exactly (by a name, an alias, a short name or its domain), if one.
    fn org_exactly(&self, name: &str) -> anyhow::Result<Option<String>> {
        let key = org_key(name);
        if key.chars().count() < 2 {
            return Ok(None);
        }
        let short = short_names(&self.paths);
        let mut st =
            self.db.prepare("select id, name, aliases from nodes where kind = 'org' and id not like 'org:~%'")?;
        let rows: Vec<(String, String, String)> =
            st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
        let hits: Vec<String> = rows
            .into_iter()
            .filter(|(id, n, a)| {
                let label = id.strip_prefix("org:").and_then(|d| d.split('.').next()).unwrap_or("").to_string();
                std::iter::once(n.clone())
                    .chain(std::iter::once(label))
                    .chain(serde_json::from_str::<Vec<String>>(a).unwrap_or_default())
                    .chain(short.get(id).cloned().unwrap_or_default())
                    .any(|x| org_key(&x) == key)
            })
            .map(|(id, ..)| id)
            .collect();
        Ok((hits.len() == 1).then(|| hits[0].clone()))
    }

    /// The matters this question asks the graph about, as hits: the user's own and the latest first. A name the
    /// question gives as a person or a word that is exactly one organization is asked about as that organization.
    pub fn graph_lookup(&mut self, a: &Ask1, day: &str, limit: usize) -> anyhow::Result<Vec<Value>> {
        let mut a = a.clone();
        let mut people = Vec::new();
        for p in std::mem::take(&mut a.people) {
            if self.nodes_named(&p, &["person"])?.is_empty()
                && let Some(_) = self.org_exactly(&p)?
            {
                a.orgs.push(p);
            } else {
                people.push(p);
            }
        }
        a.people = people;
        // a word naming an organization, when no one is named (with a person named, a word tells which one: #9)
        for w in std::mem::take(&mut a.words) {
            if a.people.is_empty() && self.org_exactly(&w)?.is_some() && !a.orgs.contains(&w) {
                a.orgs.push(w);
            }
        }
        let a = &a;
        if a.is_empty() {
            return Ok(Vec::new());
        }
        self.ready_graph()?;
        let groups = groups(&self.paths);
        // who and what
        let mut ids = Vec::new();
        // sure: every organization asked about is exactly one known by its mail domain, and nothing else is asked
        let mut sure = !a.orgs.is_empty() && a.things.is_empty() && a.roles.is_empty() && a.states.is_empty();
        for o in &a.orgs {
            sure &= self.org_exactly(o)?.is_some();
            let named = self.orgs_named(o)?;
            ids.extend(self.with_same(named)?);
        }
        for t in &a.things {
            ids.extend(self.nodes_named(t, &["thing", "case", "doc"])?);
        }
        let mut person_ids = Vec::new();
        for p in &a.people {
            person_ids.extend(self.nodes_named(p, &["person"])?);
        }
        // a name asked about that is no one in the graph finds nothing (not every matter)
        let named = !(a.orgs.is_empty() && a.things.is_empty() && (a.people.is_empty() || !a.roles.is_empty()));
        let entity: Option<HashSet<String>> =
            if named { Some(self.cases_with(&[ids, person_ids.clone()].concat())?) } else { None };
        // by role
        let role: Option<HashSet<String>> = if a.roles.is_empty() {
            None
        } else {
            let mut st = self.db.prepare("select src, dst, label from edges where rel = 'role'")?;
            let rows: Vec<(String, String, String)> =
                st.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<rusqlite::Result<_>>()?;
            Some(
                rows.into_iter()
                    .filter(|(src, _, l)| {
                        label_fits(l, &a.roles, &groups) && (person_ids.is_empty() || person_ids.contains(src))
                    })
                    .map(|(_, dst, _)| dst)
                    .collect(),
            )
        };
        let mut cases: HashSet<String> = match (entity, role) {
            (Some(e), Some(r)) => {
                let both: HashSet<String> = e.intersection(&r).cloned().collect();
                if both.is_empty() { r } else { both }
            }
            (Some(e), None) => e,
            (None, Some(r)) => r,
            (None, None) => {
                let mut st =
                    self.db.prepare("select distinct src from edges where rel = 'state' and src like 'case:%'")?;
                let mut all: HashSet<String> = st.query_map([], |r| r.get::<_, String>(0))?.flatten().collect();
                if a.open_requests {
                    let mut st = self.db.prepare(
                        "select distinct d.case_key from edges e join docs d on d.n = e.n where e.rel = 'requested' and d.case_key is not null",
                    )?;
                    all.extend(st.query_map([], |r| r.get::<_, String>(0))?.flatten());
                }
                all
            }
        };
        // how they stand
        if !a.states.is_empty() || !a.absent.is_empty() {
            cases.retain(|c| {
                let mut st = match self.db.prepare(
                    "select label, since from edges where src = ? and rel = 'state' and since <= ? order by since",
                ) {
                    Ok(st) => st,
                    Err(_) => return false,
                };
                let states: Vec<(String, String)> = st
                    .query_map(params![c, day], |r| Ok((r.get(0)?, r.get(1)?)))
                    .map(|r| r.flatten().collect())
                    .unwrap_or_default();
                let hit = if a.states.is_empty() {
                    Some(0)
                } else {
                    states.iter().rposition(|(l, _)| label_fits(l, &a.states, &groups))
                };
                match hit {
                    None => false,
                    Some(i) => {
                        !states[i..].iter().any(|(l, _)| !a.absent.is_empty() && label_fits(l, &a.absent, &groups))
                    }
                }
            });
        }
        // requests the user did not answer: no mail of theirs in the matter after the request
        if a.open_requests {
            let me = mine(&self.db)?;
            cases.retain(|c| {
                let asked: Option<String> = self
                    .db
                    .query_row(
                        "select max(e.since) from edges e join docs d on d.n = e.n where e.rel = 'requested' and d.case_key = ?",
                        [c],
                        |r| r.get(0),
                    )
                    .ok()
                    .flatten();
                let Some(asked) = asked else { return false };
                let answered: i64 = self
                    .db
                    .query_row(
                        "select count(*) from docs where case_key = ? and kind = '보낸 메일' and day >= ?",
                        params![c, asked],
                        |r| r.get(0),
                    )
                    .unwrap_or(0);
                answered == 0 && !me.is_empty()
            });
        }
        // the user's own and the latest first, up to the day asked
        let mut ranked: Vec<(i64, String, i64, String)> = Vec::new();
        for c in &cases {
            let row: Option<(i64, String, i64)> = self
                .db
                .query_row(
                    "select coalesce(max(involved), 0), max(day), (select n from docs where case_key = ?1 and day <= ?2 and source != 'file' order by day desc, n desc limit 1)
                     from docs where case_key = ?1 and day <= ?2",
                    params![c, day],
                    |r| Ok((r.get(0)?, r.get::<_, Option<String>>(1)?.unwrap_or_default(), r.get::<_, Option<i64>>(2)?.unwrap_or(0))),
                )
                .optional()?;
            if let Some((inv, last, n)) = row
                && n > 0
            {
                ranked.push((inv, last, n, c.clone()));
            }
        }
        ranked.sort_by(|x, y| y.0.cmp(&x.0).then(y.1.cmp(&x.1)));
        let words: Vec<String> = a.orgs.iter().chain(&a.things).chain(&a.people).chain(&a.roles).cloned().collect();
        let mut out = Vec::new();
        for (_, _, n, c) in ranked.into_iter().take(limit) {
            let mut h = self.hit(n, Some(&c), &words.join(" "))?;
            if let Some((l, since)) = self.last_state(&c, day) {
                h["state"] = json!(format!("{l} ({since})"));
            }
            let roles: Vec<String> = {
                let mut st = self.db.prepare(
                    "select coalesce(n.name, e.src), e.label from edges e left join nodes n on n.id = e.src where e.dst = ? and e.rel = 'role' limit 4",
                )?;
                st.query_map([&c], |r| Ok(format!("{} {}", r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
                    .flatten()
                    .collect()
            };
            if !roles.is_empty() {
                h["roles"] = json!(roles.join(", "));
            }
            if sure {
                h["sure"] = json!(true); // the matters of the one asked about: before what the words find
            }
            out.push(h);
        }
        Ok(out)
    }

    fn ready_graph(&self) -> anyhow::Result<()> {
        let v: Option<String> =
            self.db.query_row("select value from meta where key = 'schema'", [], |r| r.get(0)).optional()?;
        if v.as_deref() != Some(super::index::SCHEMA_VERSION) {
            anyhow::bail!(super::index::Outdated("index of another version: run update".into()));
        }
        Ok(())
    }
}

/// Put the labels the model gave roles and states not grouped yet into groups of the same meaning (the model
/// decides; titles and ranks are no roles). Kept beside the answers. -> labels grouped
pub fn group_labels(lib: &Librarian, model: &dyn Model) -> anyhow::Result<usize> {
    let path = cache_dir(&lib.paths).join("labels.json");
    let mut known: std::collections::HashMap<String, String> = groups(&lib.paths);
    let mut st = lib.db.prepare("select distinct label from edges where rel in ('role', 'state') and label != ''")?;
    let new: Vec<String> =
        st.query_map([], |r| r.get::<_, String>(0))?.flatten().filter(|l| !known.contains_key(l)).take(300).collect();
    if new.is_empty() {
        return Ok(0);
    }
    let names: Vec<&String> = {
        let mut v: Vec<&String> = known.values().collect();
        v.sort();
        v.dedup();
        v
    };
    let system = "업무 기록에서 뽑은 역할·상태 이름들을 같은 뜻끼리 묶는다. 이미 있는 묶음 이름이 맞으면 그것을 쓰고, 없으면 짧은 새 이름을 만든다. 뜻이 다르면 묶지 않는다. JSON 만: {\"g\": {\"원래 이름\": \"묶음 이름\"}}";
    let user = format!(
        "[이미 있는 묶음]\n{}\n\n[묶을 이름]\n{}",
        names.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("\n"),
        new.join("\n")
    );
    let answer = model.complete(
        &[json!({"role": "system", "content": system}), json!({"role": "user", "content": user})],
        Ask::new(0.0, 4000, true, 300),
    )?;
    let v = crate::llm::parse_json(&answer)?;
    let mut n = 0;
    if let Some(m) = v.get("g").and_then(Value::as_object) {
        for l in &new {
            if let Some(g) = m.get(l).and_then(Value::as_str).filter(|g| !g.trim().is_empty()) {
                known.insert(l.clone(), g.trim().chars().take(40).collect());
                n += 1;
            }
        }
    }
    crate::fsutil::ensure_private_dir(&cache_dir(&lib.paths))?;
    crate::fsutil::write_private_atomic(&path, json!(known).to_string().as_bytes())?;
    Ok(n)
}

// ---------- what the user corrected (#47) ----------

/// The key a relation is corrected by: its ends, kind and record (stable across rebuilds).
pub fn edge_key(e: &Edge) -> String {
    let s = format!("{}|{}|{}|{}", e.src, e.rel, e.dst, e.doc);
    hex::encode(&Sha256::digest(s.as_bytes())[..8])
}

/// Apply every correction of the graph kept so far (idempotent): the user's names for organizations, organizations
/// said to be one, relations taken away or renamed.
pub fn apply_fixes(db: &Connection, paths: &Paths) -> anyhow::Result<usize> {
    use crate::assistant::corrections::{Ref, of_graph};
    let fixes = of_graph(paths);
    if fixes.is_empty() {
        return Ok(0);
    }
    let edges: Vec<(i64, Edge)> = {
        let mut st = db.prepare("select rowid, src, rel, dst, label, since, file, evidence, origin from edges")?;
        st.query_map([], |r| {
            Ok((
                r.get(0)?,
                Edge {
                    src: r.get(1)?,
                    rel: r.get(2)?,
                    dst: r.get(3)?,
                    label: r.get(4)?,
                    since: r.get(5)?,
                    doc: r.get(6)?,
                    evidence: r.get(7)?,
                    origin: r.get(8)?,
                },
            ))
        })?
        .collect::<rusqlite::Result<_>>()?
    };
    let by_key: std::collections::HashMap<String, i64> = edges.iter().map(|(id, e)| (edge_key(e), *id)).collect();
    let mut done = 0;
    for f in &fixes {
        match &f.refers {
            Ref::Org(d) => {
                let id = format!("org:{d}");
                match f.right.strip_prefix("org:") {
                    Some(other) => {
                        let other = format!("org:{other}");
                        db.execute(
                            "insert or ignore into org_same(a, b) values (?1, ?2), (?2, ?1)",
                            params![id, other],
                        )?;
                    }
                    None => add_alias(db, &id, &f.right)?,
                }
                done += 1;
            }
            Ref::Edge(k) => {
                if let Some(rowid) = by_key.get(k) {
                    if f.right == "-" {
                        db.execute("delete from edges where rowid = ?", [rowid])?;
                    } else {
                        db.execute("update edges set label = ? where rowid = ?", params![f.right, rowid])?;
                    }
                    done += 1;
                }
            }
            _ => {}
        }
    }
    Ok(done)
}

impl Librarian {
    /// Is what this correction of the graph is about there (an organization, the one it is said to be, a relation)?
    pub fn graph_fix_fits(&self, c: &crate::assistant::corrections::Correction) -> bool {
        use crate::assistant::corrections::Ref;
        let org = |d: &str| self.node(&format!("org:{d}")).is_some();
        match &c.refers {
            Ref::Org(d) => org(d) && c.right.strip_prefix("org:").is_none_or(|o| o != d && org(o)),
            Ref::Edge(k) => {
                let mut st =
                    match self.db.prepare("select src, rel, dst, label, since, file, evidence, origin from edges") {
                        Ok(st) => st,
                        Err(_) => return false,
                    };
                st.query_map([], |r| {
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
                })
                .map(|rows| rows.flatten().any(|e| edge_key(&e) == *k))
                .unwrap_or(false)
            }
            _ => false,
        }
    }

    /// Apply the graph's corrections now (they are applied on every update too).
    pub fn apply_graph_fixes(&mut self) -> anyhow::Result<usize> {
        self.in_write(|me| apply_fixes(&me.db, &me.paths))
    }

    /// The organizations said to be one with these (and these).
    fn with_same(&self, ids: Vec<String>) -> anyhow::Result<Vec<String>> {
        let mut out = ids.clone();
        for id in &ids {
            let mut st = self.db.prepare("select b from org_same where a = ?")?;
            for b in st.query_map([id], |r| r.get::<_, String>(0))?.flatten() {
                if !out.contains(&b) {
                    out.push(b);
                }
            }
        }
        Ok(out)
    }
}

// ---------- what the graph screen reads (desktop) ----------

impl Librarian {
    /// Nodes a name finds (organizations by any of their names, people, matters, things), the ones with most
    /// relations first: `[{id, kind, name, aliases, edges}]`.
    pub fn graph_search(&self, q: &str, limit: usize) -> anyhow::Result<Vec<Value>> {
        self.ready_graph()?;
        let q = q.trim();
        if q.chars().count() < 2 {
            return Ok(Vec::new());
        }
        let mut ids = self.with_same(self.orgs_named(q)?)?;
        if let Some(o) = self.org_exactly(q)? {
            ids.insert(0, o);
        }
        ids.extend(self.nodes_named(q, &["person", "case", "thing", "doc", "todo", "event"])?);
        let mut seen = HashSet::new();
        ids.retain(|i| seen.insert(i.clone()));
        let mut out: Vec<(i64, Value)> = Vec::new();
        for id in ids {
            let Some(n) = self.node(&id) else { continue };
            let count: i64 =
                self.db.query_row("select count(*) from edges where src = ?1 or dst = ?1", [&id], |r| r.get(0))?;
            out.push((
                count,
                json!({"id": n.id, "kind": n.kind, "name": n.name, "aliases": n.aliases, "edges": count}),
            ));
        }
        out.sort_by_key(|b| std::cmp::Reverse(b.0));
        Ok(out.into_iter().take(limit).map(|(_, v)| v).collect())
    }

    /// One node and its relations, each with the other end's name, the record it came from and its key to correct
    /// it by: `{node, same, edges: [{key, rel, dir, other, other_name, label, since, evidence, origin, title, day}]}`.
    pub fn graph_node(&self, id: &str) -> anyhow::Result<Option<Value>> {
        self.ready_graph()?;
        let Some(n) = self.node(id) else { return Ok(None) };
        let short = short_names(&self.paths).remove(id).unwrap_or_default();
        let same = self.with_same(vec![id.to_string()])?.into_iter().skip(1).collect::<Vec<_>>();
        let mut edges = Vec::new();
        for (dir, list) in [("out", self.edges_from(id, None)), ("in", self.edges_to(id, None))] {
            for e in list.into_iter().take(300) {
                let other = if dir == "out" { e.dst.clone() } else { e.src.clone() };
                let other_name =
                    self.node(&other).map(|o| o.name).filter(|n| !n.is_empty()).unwrap_or_else(|| other.clone());
                let (title, day): (String, String) = self
                    .db
                    .query_row("select title, day from docs where file = ? order by n limit 1", [&e.doc], |r| {
                        Ok((r.get(0)?, r.get(1)?))
                    })
                    .optional()?
                    .unwrap_or_default();
                edges.push(
                    json!({"key": edge_key(&e), "rel": e.rel, "dir": dir, "other": other, "other_name": other_name,
                                  "label": e.label, "since": e.since, "evidence": e.evidence, "origin": e.origin,
                                  "title": title, "day": day}),
                );
            }
        }
        Ok(Some(json!({"node": {"id": n.id, "kind": n.kind, "name": n.name, "aliases": n.aliases, "short": short},
                       "same": same, "edges": edges})))
    }
}
