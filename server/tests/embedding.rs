//! Embedding finds the matter a question is about (ADR 0007): vectors kept in the index, made only for new and changed
//! documents, searched by meaning one hit per matter, and the words searched instead whenever there is no embedding
//! server or no vector yet. Synthetic records and a fake embedder only.
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use life_server::assistant::recall::{self, Named, Picked};
use life_server::config::Paths;
use life_server::host::Models;
use life_server::librarian::vectors::{EMBED_BATCH, EMBED_INSTRUCT};
use life_server::librarian::{Librarian, Outdated};
use life_server::llm::{Embedder, WordsEmbedder};
use serde_json::Value;

struct T {
    _dir: tempfile::TempDir,
    p: Paths,
    lib: Librarian,
}

impl T {
    fn new() -> T {
        let dir = tempfile::tempdir().unwrap();
        let p = Paths::under(dir.path());
        for d in [p.vault.join("Mail"), p.raw().join("lorelog")] {
            fs::create_dir_all(d).unwrap();
        }
        let lib = Librarian::open(&p, None, Duration::from_secs(5)).unwrap();
        T { _dir: dir, p, lib }
    }

    fn mail(&self, name: &str, subject: &str, body: &str, day: &str) -> PathBuf {
        self.mail_in(name, subject, body, day, &format!("<{name}@x>"), "[]", "")
    }

    #[allow(clippy::too_many_arguments)]
    fn mail_in(&self, name: &str, subject: &str, body: &str, day: &str, id: &str, refs: &str, reply: &str) -> PathBuf {
        let path = self.p.vault.join("Mail").join(&day[..4]).join(name);
        write(
            &path,
            &format!(
                "---\ntype: mail\nmail_id: {id}\nmail_source: account/INBOX\nmail_synced: 2024-01-01\ndate: {day}\ntime: 09:00\nfrom: 철수 <a@x.com>\nto: 나 <me@x.com>\nsubject: \"{subject}\"\nin_reply_to: {reply}\nreferences: {refs}\nattachments: []\n---\n## Notes\n\n## Message\n{body}\n"
            ),
        );
        path
    }

    fn update(&mut self) {
        self.lib.update(false).unwrap();
    }

    /// Three matters: one of two mails about a mould quote, a meeting, a trip.
    fn world(&mut self) {
        let root = "<root-mould@x>";
        self.mail_in(
            "2024-04-01-0900-q1.md",
            "금형 견적 요청",
            "금형 단가 견적 부탁드립니다",
            "2024-04-01",
            root,
            "[]",
            "",
        );
        self.mail_in(
            "2024-04-03-0900-q2.md",
            "RE: 금형 견적 요청",
            "금형 단가 견적 보내드립니다",
            "2024-04-03",
            "<r1-mould@x>",
            &format!("[{root}]"),
            root,
        );
        self.mail("2024-04-02-0900-m.md", "회의 일정 공유", "다음 주 회의 일정 공유합니다", "2024-04-02");
        self.mail("2024-04-04-0900-t.md", "출장 보고", "부산 출장 결과 보고드립니다", "2024-04-04");
        self.update();
    }
}

fn write(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A fake that counts the texts it was asked for, and fails the calls `fail` says.
struct Counting {
    inner: WordsEmbedder,
    calls: AtomicUsize,
    texts: Mutex<Vec<String>>,
    fail: Box<dyn Fn(usize) -> bool + Send + Sync>,
}

impl Counting {
    fn new(fail: impl Fn(usize) -> bool + Send + Sync + 'static) -> Counting {
        Counting {
            inner: WordsEmbedder::new("fake"),
            calls: AtomicUsize::new(0),
            texts: Mutex::new(Vec::new()),
            fail: Box::new(fail),
        }
    }

    fn ok() -> Counting {
        Counting::new(|_| false)
    }
}

impl Embedder for Counting {
    fn model(&self) -> String {
        self.inner.model()
    }

    fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        if (self.fail)(call) {
            anyhow::bail!("server down");
        }
        self.texts.lock().unwrap().extend(texts.iter().cloned());
        self.inner.embed(texts)
    }
}

fn titles(hits: &[Value]) -> Vec<String> {
    hits.iter().map(|h| h["title"].as_str().unwrap_or("").to_string()).collect()
}

fn query(e: &dyn Embedder, q: &str) -> Vec<f32> {
    e.embed(&[format!("{EMBED_INSTRUCT}{q}")]).unwrap().remove(0)
}

// (a) new and changed documents only
#[test]
fn documents_are_embedded_once_and_again_only_when_their_text_changes() {
    let mut t = T::new();
    t.world();
    let e = Counting::ok();
    let r = t.lib.embed_missing(&e).unwrap();
    assert_eq!((r.embedded, r.failed, r.stopped), (4, 0, false));
    assert_eq!(t.lib.vector_count("fake"), 4);
    // the document's text is its title and the start of its body; the question's instruction is not on it
    let texts = e.texts.lock().unwrap().clone();
    assert!(texts.iter().any(|x| x == "출장 보고\n부산 출장 결과 보고드립니다"), "{texts:?}");
    assert!(texts.iter().all(|x| !x.contains("Instruct:")));

    let again = t.lib.embed_missing(&e).unwrap();
    assert_eq!((again.embedded, again.skipped), (0, 4));
    assert_eq!(e.calls.load(Ordering::SeqCst), 1, "nothing new: the server is not asked");

    std::thread::sleep(Duration::from_millis(20));
    t.mail("2024-04-04-0900-t.md", "출장 보고", "부산 출장 결과 수정본입니다", "2024-04-04");
    t.update();
    let changed = t.lib.embed_missing(&e).unwrap();
    assert_eq!((changed.embedded, changed.skipped), (1, 3));
    assert_eq!(t.lib.vector_count("fake"), 4, "the old text's vector went with it");
}

#[test]
fn a_long_body_is_cut_to_its_first_1500_letters() {
    let mut t = T::new();
    t.mail("2024-04-01-0900-long.md", "긴 메일", &"가".repeat(3000), "2024-04-01");
    t.update();
    let e = Counting::ok();
    assert_eq!(t.lib.embed_missing(&e).unwrap().embedded, 1);
    let texts = e.texts.lock().unwrap().clone();
    assert_eq!(texts[0].chars().count(), "긴 메일\n".chars().count() + 1500);
}

// (b) a server that fails
#[test]
fn a_failing_server_leaves_the_index_whole_and_three_failures_in_a_row_stop_the_run() {
    let mut t = T::new();
    for i in 0..(EMBED_BATCH * 4 + 2) {
        t.mail(&format!("2024-05-01-0900-{i}.md"), &format!("제목{i}"), &format!("본문{i} 내용"), "2024-05-01");
    }
    t.update();
    let down = Counting::new(|_| true);
    let r = t.lib.embed_missing(&down).unwrap();
    assert_eq!(down.calls.load(Ordering::SeqCst), 3, "stops after three failures in a row");
    assert!(r.stopped);
    assert_eq!((r.embedded, r.failed), (0, EMBED_BATCH * 3));
    assert_eq!(t.lib.vector_count("fake"), 0);
    assert!(!t.lib.search("본문3", 5, None).unwrap().is_empty(), "the index is searched as before");

    // one batch fails, the others go on; what failed is tried again next time
    let flaky = Counting::new(|call| call == 0);
    let r = t.lib.embed_missing(&flaky).unwrap();
    assert_eq!((r.embedded, r.failed, r.stopped), (EMBED_BATCH * 3 + 2, EMBED_BATCH, false));
    let r = t.lib.embed_missing(&Counting::ok()).unwrap();
    assert_eq!(r.embedded, EMBED_BATCH);
}

#[test]
fn a_server_answering_with_the_wrong_number_of_vectors_is_a_failure() {
    struct Short;
    impl Embedder for Short {
        fn model(&self) -> String {
            "fake".into()
        }
        fn embed(&self, _texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
            Ok(vec![vec![1.0, 0.0]])
        }
    }
    let mut t = T::new();
    t.world();
    let r = t.lib.embed_missing(&Short).unwrap();
    assert_eq!((r.embedded, r.failed), (0, 4));
    assert_eq!(t.lib.vector_count("fake"), 0);
}

// (c) search by meaning
#[test]
fn the_nearest_matter_comes_first_and_a_matter_comes_back_once() {
    let mut t = T::new();
    t.world();
    let e = WordsEmbedder::new("fake");
    t.lib.embed_missing(&e).unwrap();
    let hits = t.lib.semantic("fake", &query(&e, "금형 단가 견적"), 8).unwrap();
    assert!(titles(&hits)[0].contains("금형 견적 요청"), "{:?}", titles(&hits));
    assert_eq!(titles(&hits).iter().filter(|t| t.contains("금형")).count(), 1, "one hit per matter");
    assert_eq!(hits[0]["count"], 2, "the hit says the matter's size as a word hit does");
    assert!(hits[0]["case"].is_string() && hits[0]["ref"].is_string());
    let scores: Vec<f64> = hits.iter().map(|h| h["score"].as_f64().unwrap()).collect();
    assert!(scores.windows(2).all(|w| w[0] >= w[1]), "{scores:?}");
    assert_eq!(t.lib.semantic("fake", &query(&e, "출장 보고"), 1).unwrap().len(), 1);
    assert!(titles(&t.lib.semantic("fake", &query(&e, "부산 출장"), 1).unwrap())[0].contains("출장"));
}

#[test]
fn a_new_vector_is_searched_after_the_next_embedding() {
    let mut t = T::new();
    t.world();
    let e = WordsEmbedder::new("fake");
    t.lib.embed_missing(&e).unwrap();
    assert!(!titles(&t.lib.semantic("fake", &query(&e, "고래 관찰"), 8).unwrap()).iter().any(|x| x.contains("고래")));
    t.mail("2024-04-09-0900-w.md", "고래 관찰 계획", "고래 관찰 일정", "2024-04-09");
    t.update();
    t.lib.embed_missing(&e).unwrap();
    assert!(titles(&t.lib.semantic("fake", &query(&e, "고래 관찰"), 8).unwrap())[0].contains("고래"));
}

// (d) the bridge: meaning when it can, words when it cannot
fn picked() -> Picked {
    Picked {
        words: vec!["출장".into()], question: "금형 단가 견적 어떻게 됐지".into(), ..Picked::default()
    }
}

fn first(lib: &mut Librarian) -> String {
    match recall::lookup(lib, &picked(), 8, "2024-12-31", &[]).unwrap() {
        Named::Hits(h) => titles(&h).first().cloned().unwrap_or_default(),
        Named::Ask(_) => "asked".into(),
    }
}

#[test]
fn the_lookup_finds_by_meaning_with_vectors_and_by_the_words_without() {
    let mut t = T::new();
    t.world();
    assert_eq!(first(&mut t.lib), "출장 보고", "no embedder: the words");
    t.lib.embedder = Some(Arc::new(WordsEmbedder::new("fake")));
    assert_eq!(first(&mut t.lib), "출장 보고", "no vector yet: the words");
    t.lib.embed_missing(&WordsEmbedder::new("fake")).unwrap();
    assert!(first(&mut t.lib).contains("금형 견적 요청"), "by meaning");
    t.lib.embedder = Some(Arc::new(Counting::new(|_| true)));
    assert_eq!(first(&mut t.lib), "출장 보고", "the server down: the words");
    t.lib.embedder = Some(Arc::new(WordsEmbedder::new("other")));
    assert_eq!(first(&mut t.lib), "출장 보고", "vectors of another model are not compared");
}

#[test]
fn the_picker_keeps_the_question_for_the_embedding() {
    let m = life_server::llm::Http { url: String::new(), model: String::new() };
    let p = recall::pick(&m, "금형 견적 어떻게 됐지", None).unwrap();
    assert_eq!(p.question, "금형 견적 어떻게 됐지");
}

// (e) the schema
#[test]
fn an_index_of_the_version_before_is_built_again_and_then_embedded() {
    let mut t = T::new();
    t.world();
    let path = t.lib.path.clone();
    drop(t.lib);
    {
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch("update meta set value = '17' where key = 'schema'; drop table if exists vectors;").unwrap();
    }
    let mut lib = Librarian::open(&t.p, None, Duration::from_secs(5)).unwrap();
    let e = lib.search("출장", 5, None).unwrap_err();
    assert!(e.downcast_ref::<Outdated>().is_some());
    lib.update(false).unwrap();
    assert_eq!(lib.search("출장", 5, None).unwrap().len(), 1);
    assert_eq!(lib.embed_missing(&WordsEmbedder::new("fake")).unwrap().embedded, 4);
}

#[test]
fn vectors_live_in_the_index_file_private_and_go_with_their_records() {
    let mut t = T::new();
    t.world();
    let listed = |t: &T| -> Vec<String> {
        let mut v: Vec<String> =
            fs::read_dir(&t.p.data).unwrap().flatten().map(|f| f.file_name().to_string_lossy().into()).collect();
        v.sort();
        v
    };
    let before = listed(&t);
    t.lib.embed_missing(&WordsEmbedder::new("fake")).unwrap();
    assert_eq!(listed(&t), before, "no file of their own");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&t.lib.path).unwrap().permissions().mode() & 0o777, 0o600);
    }
    // a rebuilt index keeps the vectors of the same texts (nothing to ask again) ...
    t.lib.update(true).unwrap();
    assert_eq!(t.lib.embed_missing(&WordsEmbedder::new("fake")).unwrap().embedded, 0);
    // ... and a removed record takes its vector with it on the next update
    fs::remove_file(t.p.vault.join("Mail/2024/2024-04-04-0900-t.md")).unwrap();
    t.update();
    assert_eq!(t.lib.vector_count("fake"), 3);
    // another model's vectors are dropped when this one embeds
    t.lib.embed_missing(&WordsEmbedder::new("other")).unwrap();
    assert_eq!((t.lib.vector_count("fake"), t.lib.vector_count("other")), (0, 3));
}

// (f) off by default
#[test]
fn embedding_is_off_unless_a_model_is_named() {
    let file = [("LIFE_LLM_URL".to_string(), "http://127.0.0.1:9/v1".to_string())].into();
    let none = Models::from_maps(&file, &Default::default());
    assert!(none.embedder().is_none());
    let env = [("LIFE_EMBED_MODEL".to_string(), "m".to_string())].into();
    let some = Models::from_maps(&file, &env);
    assert_eq!(some.embed_model, "m");
    assert_eq!(some.embedder().map(|e| e.model()).as_deref(), Some("m"));
    let no_url = Models::from_maps(&Default::default(), &env);
    assert!(no_url.embedder().is_none(), "no server: off");
}

#[test]
fn an_embedding_server_not_set_or_not_there_is_an_error() {
    let none = life_server::llm::HttpEmbedder { url: String::new(), model: "m".into() };
    assert!(none.embed(&["a".into()]).is_err());
    let gone = life_server::llm::HttpEmbedder { url: "http://127.0.0.1:9/v1".into(), model: "m".into() };
    assert!(gone.embed(&["a".into()]).is_err());
    assert!(gone.embed(&[]).unwrap().is_empty(), "nothing to embed asks nothing");
}

#[test]
fn saving_the_settings_screen_keeps_the_embedding_model() {
    let dir = tempfile::tempdir().unwrap();
    let p = Paths::under(dir.path());
    fs::create_dir_all(&p.conf).unwrap();
    let env = life_server::host::ingest_env(&p);
    fs::write(&env, "LIFE_EMBED_MODEL=m\n").unwrap();
    let screen = Models { chat_url: "http://127.0.0.1:9/v1".into(), ..Models::default() };
    life_server::host::save_models(&p, &screen).unwrap();
    assert_eq!(life_server::host::saved_models(&p).embed_model, "m");
}

// ---------- the server's answer, over real HTTP ----------

/// A one-request server on a free local port answering `body` as JSON; -> its address as `LIFE_LLM_URL` gives it.
fn stub(body: String) -> String {
    use std::io::{BufRead, BufReader, Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || {
        let Ok((stream, _)) = listener.accept() else { return };
        let mut reader = BufReader::new(stream);
        let mut len = 0;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
        let mut sent = vec![0; len];
        let _ = reader.read_exact(&mut sent);
        let mut stream = reader.into_inner();
        let _ = write!(
            stream,
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
    });
    format!("http://{addr}/v1")
}

fn ask_stub(body: &str, n: usize) -> anyhow::Result<Vec<Vec<f32>>> {
    let e = life_server::llm::HttpEmbedder { url: stub(body.to_string()), model: "m".into() };
    let texts: Vec<String> = (0..n).map(|i| format!("t{i}")).collect();
    e.embed(&texts)
}

#[test]
fn the_servers_vectors_come_back_in_the_order_of_the_texts() {
    let ok = r#"{"data": [{"index": 0, "embedding": [1, 0]}, {"index": 1, "embedding": [0, 1]}]}"#;
    assert_eq!(ask_stub(ok, 2).unwrap(), vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
    let shuffled = r#"{"data": [{"index": 1, "embedding": [0, 1]}, {"index": 0, "embedding": [1, 0]}]}"#;
    assert_eq!(ask_stub(shuffled, 2).unwrap(), vec![vec![1.0, 0.0], vec![0.0, 1.0]]);
}

#[test]
fn an_answer_that_is_not_one_good_vector_per_text_is_an_error() {
    let cases = [
        ("count", r#"{"data": [{"index": 0, "embedding": [1, 0]}]}"#.to_string()),
        ("empty", r#"{"data": [{"index": 0, "embedding": []}, {"index": 1, "embedding": [1]}]}"#.to_string()),
        ("no data", r#"{"object": "list"}"#.to_string()),
        (
            "not a number",
            r#"{"data": [{"index": 0, "embedding": [1, "x"]}, {"index": 1, "embedding": [0, 1]}]}"#.to_string(),
        ),
        // too large for f32: infinite once it is one
        (
            "infinite",
            r#"{"data": [{"index": 0, "embedding": [1e300, 0]}, {"index": 1, "embedding": [0, 1]}]}"#.to_string(),
        ),
        ("too long", {
            let long = vec!["0.1"; life_server::llm::MAX_DIM + 1].join(",");
            format!(r#"{{"data": [{{"index": 0, "embedding": [{long}]}}, {{"index": 1, "embedding": [{long}]}}]}}"#)
        }),
    ];
    for (what, body) in cases {
        assert!(ask_stub(&body, 2).is_err(), "{what}");
    }
}

/// A fake answering the same vector for every text.
struct Fixed(Vec<f32>);

impl Embedder for Fixed {
    fn model(&self) -> String {
        "fake".into()
    }
    fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(texts.iter().map(|_| self.0.clone()).collect())
    }
}

#[test]
fn vectors_that_are_not_finite_or_too_long_are_not_kept() {
    let mut t = T::new();
    t.world();
    for bad in [vec![f32::NAN, 1.0], vec![f32::INFINITY, 1.0], vec![0.1; life_server::llm::MAX_DIM + 1]] {
        let r = t.lib.embed_missing(&Fixed(bad)).unwrap();
        assert_eq!((r.embedded, r.failed), (0, 4));
    }
    assert_eq!(t.lib.vector_count("fake"), 0);
}

#[test]
fn a_damaged_vector_is_passed_over_and_the_others_are_searched() {
    let mut t = T::new();
    t.world();
    let e = WordsEmbedder::new("fake");
    t.lib.embed_missing(&e).unwrap();
    {
        let db = rusqlite::Connection::open(&t.lib.path).unwrap();
        db.execute(
            "update vectors set vec = x'00000000' where hash = (select ehash from docs where title = '출장 보고')",
            [],
        )
        .unwrap();
    }
    let hits = t.lib.semantic("fake", &query(&e, "부산 출장 보고"), 8).unwrap();
    let ts = titles(&hits);
    assert!(!ts.iter().any(|x| x.contains("출장")), "{ts:?}");
    assert_eq!(ts.len(), 2, "the mould matter and the meeting");
}
