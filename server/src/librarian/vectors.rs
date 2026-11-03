//! What the librarian keeps to find the matter a question is about by meaning (ADR 0007): one vector per document text
//! (mails, the files that came with them, events; not the phone's traces nor the user's words), made by the user's own
//! embedding model and kept in the index, so the index's rules hold for them too (mode 600, a copy built from the
//! records, gone with the file).
//!
//! A vector is keyed by the model and a hash of the text it was made of (the title and the first [`EMBED_CHARS`]
//! letters of the body), not by the document's row: a document indexed again with the same text, after a rebuild or a
//! schema change, needs no new vector. Every update that changes the documents drops the vectors no document has any
//! more, and a run for one model drops the other models' vectors. Only the texts without a vector are sent.
//!
//! A search compares the question's vector with every document's (dot product of unit vectors), folds the documents of
//! one matter into its best one and hands back the hits as the word search does. The vectors are read once into memory
//! per process and read again when an update or an embedding run changed them (a token in `meta`).
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use rusqlite::{Connection, params};
use serde_json::{Value, json};

use super::index::{Bound, Librarian};
use super::records::Doc;
use crate::llm::Embedder;

/// Texts sent in one request.
pub const EMBED_BATCH: usize = 16;
/// Letters of a document's body that are embedded (after its title).
pub const EMBED_CHARS: usize = 1500;
/// Requests failing one after another that stop a run (the rest waits for the next).
pub const MAX_FAILURES: usize = 3;
/// What goes before a question (not before a document): the setting ADR 0007 measured.
pub const EMBED_INSTRUCT: &str = "Instruct: Given a question about the user's past work, retrieve the emails that answer it or are earlier cases like it\nQuery: ";
/// The documents that are embedded.
pub const SOURCES: [&str; 3] = ["mail", "file", "event"];

/// The text of a document as it is embedded.
pub fn embed_text(title: &str, text: &str) -> String {
    format!("{title}\n{}", text.chars().take(EMBED_CHARS).collect::<String>())
}

fn hash(text: &str) -> String {
    use sha1::Digest;
    hex::encode(sha1::Sha1::digest(text.as_bytes()))
}

/// The key of what is embedded of a document, None for a document that is not embedded.
pub(crate) fn doc_hash(d: &Doc) -> Option<String> {
    SOURCES.contains(&d.source).then(|| hash(&embed_text(&d.title, &d.text)))
}

/// What an embedding run did: documents' texts embedded now, already embedded, failed (tried again next time), and
/// whether it stopped early after [`MAX_FAILURES`] failures in a row.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub struct EmbedReport {
    pub embedded: usize,
    pub skipped: usize,
    pub failed: usize,
    pub stopped: bool,
}

/// The documents changed (an update): the vectors no document has any more go, and searches read the vectors anew.
pub(crate) fn changed(db: &Connection) -> rusqlite::Result<()> {
    db.execute("delete from vectors where hash not in (select ehash from docs where ehash is not null)", [])?;
    bump(db)
}

fn bump(db: &Connection) -> rusqlite::Result<()> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let ns = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let token = format!("{ns}-{}-{}", std::process::id(), SEQ.fetch_add(1, Ordering::Relaxed));
    db.execute("insert or replace into meta(key, value) values ('vectors', ?)", [token])?;
    Ok(())
}

/// A vector as kept: f32 little-endian.
fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_blob(b: &[u8]) -> impl Iterator<Item = f32> + '_ {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
}

/// The vector of unit length (a zero vector stays zero).
fn unit(v: &[f32]) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 { v.iter().map(|x| x / n).collect() } else { v.to_vec() }
}

/// Why an embedding request failed, in a word: never the text sent or the server's answer.
pub fn why(e: &anyhow::Error) -> String {
    if let Some(u) = e.downcast_ref::<ureq::Error>() {
        return match u {
            ureq::Error::StatusCode(c) => format!("HTTP {c}"),
            ureq::Error::Timeout(_) => "timeout".into(),
            ureq::Error::Io(io) => format!("{:?}", io.kind()),
            ureq::Error::HostNotFound => "host not found".into(),
            ureq::Error::ConnectionFailed => "connection failed".into(),
            _ => "request failed".into(),
        };
    }
    if let Some(io) = e.downcast_ref::<std::io::Error>() {
        return format!("{:?}", io.kind());
    }
    "bad answer".into()
}

/// The vectors of one model and size as searched: one row per document.
struct Matrix {
    token: Option<String>,
    model: String,
    dim: usize,
    /// (document, its matter)
    ids: Vec<(i64, Option<String>)>,
    data: Vec<f32>,
}

/// The vectors last read, per index file.
static READ: LazyLock<Mutex<HashMap<PathBuf, Arc<Matrix>>>> = LazyLock::new(|| Mutex::new(HashMap::new()));

impl Librarian {
    /// The texts with a vector of this model.
    pub fn vector_count(&self, model: &str) -> usize {
        self.db
            .query_row("select count(*) from vectors where model = ?", [model], |r| r.get::<_, i64>(0))
            .map(|n| n as usize)
            .unwrap_or(0)
    }

    pub(crate) fn has_vectors(&self, model: &str) -> bool {
        self.db
            .query_row("select exists(select 1 from vectors where model = ?)", [model], |r| r.get::<_, bool>(0))
            .unwrap_or(false)
    }

    /// Embed the documents' texts that have no vector of this model yet (see [`EmbedReport`]). A request that fails
    /// leaves its texts for the next run; the index is never left broken by it.
    pub fn embed_missing(&mut self, e: &dyn Embedder) -> anyhow::Result<EmbedReport> {
        self.embed_missing_with(e, &mut |_, _| {})
    }

    /// [`Self::embed_missing`], telling `progress(done, todo)` after every request.
    pub fn embed_missing_with(
        &mut self,
        e: &dyn Embedder,
        progress: &mut dyn FnMut(usize, usize),
    ) -> anyhow::Result<EmbedReport> {
        self.ready()?;
        let model = e.model();
        self.in_write(|me| {
            if me.db.execute("delete from vectors where model != ?", [&model])? > 0 {
                bump(&me.db)?;
            }
            Ok(())
        })?;
        // one text per hash (a mail kept twice is embedded once), the oldest documents first
        let todo: Vec<(String, i64)> = {
            let mut st = self.db.prepare(
                "select ehash, min(n) from docs d where ehash is not null
                 and not exists (select 1 from vectors v where v.model = ?1 and v.hash = d.ehash)
                 group by ehash order by min(n)",
            )?;
            st.query_map([&model], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<rusqlite::Result<_>>()?
        };
        let all: i64 =
            self.db.query_row("select count(distinct ehash) from docs where ehash is not null", [], |r| r.get(0))?;
        let mut report = EmbedReport { skipped: (all as usize).saturating_sub(todo.len()), ..EmbedReport::default() };
        let mut failures = 0;
        for batch in todo.chunks(EMBED_BATCH) {
            let mut texts = Vec::with_capacity(batch.len());
            for (_, n) in batch {
                let (title, text): (String, String) =
                    self.db
                        .query_row("select title, text from docs where n = ?", [n], |r| Ok((r.get(0)?, r.get(1)?)))?;
                texts.push(embed_text(&title, &text));
            }
            let answer = e.embed(&texts).and_then(|vs| {
                let dim = vs.first().map(Vec::len).unwrap_or(0);
                if vs.len() != texts.len() || dim == 0 || vs.iter().any(|v| v.len() != dim) {
                    anyhow::bail!("the answer does not hold one vector of one size per text");
                }
                Ok(vs)
            });
            match answer {
                Ok(vs) => {
                    self.in_write(|me| {
                        let mut st = me.db.prepare_cached(
                            "insert or replace into vectors(model, hash, dim, vec) values (?, ?, ?, ?)",
                        )?;
                        for ((h, _), v) in batch.iter().zip(&vs) {
                            st.execute(params![model, h, v.len() as i64, to_blob(&unit(v))])?;
                        }
                        bump(&me.db)?;
                        Ok(())
                    })?;
                    report.embedded += batch.len();
                    failures = 0;
                }
                Err(err) => {
                    log::warn!(
                        "librarian: embedding {} texts failed ({}); tried again next time",
                        batch.len(),
                        why(&err)
                    );
                    report.failed += batch.len();
                    failures += 1;
                }
            }
            progress(report.embedded + report.failed, todo.len());
            if failures >= MAX_FAILURES {
                report.stopped = true;
                break;
            }
        }
        if report.embedded > 0 {
            // a document gone while the server was asked: its vector does not stay
            self.in_write(|me| Ok(changed(&me.db)?))?;
        }
        Ok(report)
    }

    /// The matters whose documents are nearest in meaning to `q` (a question's vector of `model`), one hit per matter
    /// (its nearest document), the nearest first, up to `limit`; each as [`Librarian::search`] gives it, `score` the
    /// dot product of the unit vectors. Nothing when no document has a vector of that model and size.
    pub fn semantic(&mut self, model: &str, q: &[f32], limit: usize) -> anyhow::Result<Vec<Value>> {
        self.db.execute_batch("begin")?; // one read snapshot (WAL), as a word search
        let result = (|| {
            self.ready()?;
            let m = self.matrix(model, q.len())?;
            let q = unit(q);
            let mut scored: Vec<(f32, usize)> = m
                .data
                .chunks_exact(m.dim.max(1))
                .map(|row| row.iter().zip(&q).map(|(a, b)| a * b).sum())
                .zip(0..)
                .collect();
            scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
            let mut seen = HashSet::new();
            let mut out = Vec::new();
            for (score, i) in scored {
                if out.len() >= limit {
                    break;
                }
                let (n, case) = &m.ids[i];
                if !seen.insert(case.clone().unwrap_or_else(|| format!("#{n}"))) {
                    continue;
                }
                let mut hit = self.hit_within(*n, case.as_deref(), "", Bound::default())?;
                hit["score"] = json!(score as f64);
                out.push(hit);
            }
            Ok(out)
        })();
        let _ = self.db.execute_batch("rollback");
        result
    }

    /// The vectors of this model and size with their documents, as last read unless they changed since.
    fn matrix(&self, model: &str, dim: usize) -> anyhow::Result<Arc<Matrix>> {
        let token = self.meta("vectors");
        let mut read = READ.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(m) = read.get(&self.path)
            && m.token == token
            && m.model == model
            && m.dim == dim
        {
            return Ok(m.clone());
        }
        let (mut ids, mut data) = (Vec::new(), Vec::new());
        {
            let mut st = self.db.prepare(
                "select d.n, d.case_key, v.vec from docs d join vectors v on v.hash = d.ehash
                 where v.model = ?1 and v.dim = ?2 order by d.n",
            )?;
            let mut rows = st.query(params![model, dim as i64])?;
            while let Some(r) = rows.next()? {
                let blob: Vec<u8> = r.get(2)?;
                if blob.len() != dim * 4 {
                    continue;
                }
                ids.push((r.get::<_, i64>(0)?, r.get::<_, Option<String>>(1)?));
                data.extend(from_blob(&blob));
            }
        }
        let m = Arc::new(Matrix { token, model: model.to_string(), dim, ids, data });
        read.insert(self.path.clone(), m.clone());
        Ok(m)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(source: &'static str) -> Doc {
        Doc {
            source,
            kind: String::new(),
            day: "2024-01-01".into(),
            title: "t".into(),
            text: "x".into(),
            extra: String::new(),
            thread: None,
            people: Vec::new(),
            mid: None,
            uid: None,
        }
    }

    #[test]
    fn mails_files_and_events_are_embedded_and_traces_and_words_are_not() {
        for s in ["mail", "file", "event"] {
            assert!(doc_hash(&doc(s)).is_some(), "{s}");
        }
        for s in ["trace", "word"] {
            assert!(doc_hash(&doc(s)).is_none(), "{s}");
        }
    }

    #[test]
    fn a_vector_is_kept_as_little_endian_f32_of_unit_length() {
        let v = unit(&[3.0, 4.0]);
        assert_eq!(v, vec![0.6, 0.8]);
        assert_eq!(to_blob(&v)[..4], 0.6f32.to_le_bytes());
        assert_eq!(from_blob(&to_blob(&v)).collect::<Vec<_>>(), v);
    }
}
