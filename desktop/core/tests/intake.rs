//! The watched-folder intake: a file the user dropped in a chosen folder is kept by the server before the agent
//! considers it done, and nothing is ever lost or sent twice. The clock and the uploader are injected so it is tested
//! without a real server, and the folder is a temp dir.
use life_agent_core::intake::{IntakeEngine, Uploaded, UploadError, Uploader, SETTLE_S};
use std::cell::RefCell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Records every upload so a test can see what the server was asked to keep. `known` are hashes it already has.
#[derive(Default)]
struct FakeServer {
    known: RefCell<HashSet<String>>,
    puts: RefCell<Vec<(String, String, usize)>>, // (sha, name, bytes) — only real byte uploads
    sightings: RefCell<Vec<String>>,             // sha — sighting-only, no bytes
    reject: RefCell<HashSet<String>>,            // file names the server refuses for good
    fail: RefCell<bool>,
}

impl Uploader for FakeServer {
    fn has(&self, sha: &str) -> Result<bool, String> {
        if *self.fail.borrow() {
            return Err("offline".into());
        }
        Ok(self.known.borrow().contains(sha))
    }
    fn put(&self, sha: &str, name: &str, _mime: &str, data: &[u8], _when_ms: i64) -> Result<Uploaded, UploadError> {
        if *self.fail.borrow() {
            return Err(UploadError::Transient("offline".into()));
        }
        if self.reject.borrow().contains(name) {
            return Err(UploadError::Rejected("too large".into()));
        }
        let dup = self.known.borrow().contains(sha);
        self.known.borrow_mut().insert(sha.to_string());
        self.puts.borrow_mut().push((sha.to_string(), name.to_string(), data.len()));
        Ok(Uploaded { duplicate: dup })
    }
    fn sight(&self, sha: &str, _name: &str, _mime: &str, _when_ms: i64) -> Result<(), UploadError> {
        if *self.fail.borrow() {
            return Err(UploadError::Transient("offline".into()));
        }
        self.sightings.borrow_mut().push(sha.to_string());
        Ok(())
    }
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64()
}

/// A temp folder to watch plus the journal path next to it.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        Fixture { dir: tempfile::tempdir().unwrap() }
    }
    fn watch(&self) -> PathBuf {
        self.dir.path().join("watched")
    }
    fn journal(&self) -> PathBuf {
        self.dir.path().join("intake-state.json")
    }
    fn write(&self, name: &str, data: &[u8]) -> PathBuf {
        fs::create_dir_all(self.watch()).unwrap();
        let p = self.watch().join(name);
        fs::write(&p, data).unwrap();
        p
    }
    fn engine(&self) -> IntakeEngine {
        IntakeEngine::open(&self.journal()).unwrap()
    }
}

fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(data);
    format!("{:x}", h.finalize())
}

#[test]
fn a_stable_file_is_uploaded_once_the_bytes_and_a_sighting_land() {
    let fx = Fixture::new();
    fx.write("report.pdf", b"quarterly report");
    let srv = FakeServer::default();
    let mut eng = fx.engine();
    let t = now();
    // first scan notices the file but it is not yet stable (just seen)
    eng.scan(&[fx.watch()], t);
    eng.step(&srv, t);
    assert!(srv.puts.borrow().is_empty(), "a freshly seen file waits for the settle time");
    // after the settle time with no change, it uploads
    eng.scan(&[fx.watch()], t + 20.0);
    eng.step(&srv, t + 20.0);
    let puts = srv.puts.borrow();
    assert_eq!(puts.len(), 1);
    assert_eq!((puts[0].0.as_str(), puts[0].1.as_str(), puts[0].2), (sha256_hex(b"quarterly report").as_str(), "report.pdf", 16));
}

#[test]
fn a_file_still_being_written_is_not_uploaded_until_it_settles() {
    let fx = Fixture::new();
    let p = fx.write("big.zip", b"part one");
    let srv = FakeServer::default();
    let mut eng = fx.engine();
    let t = now();
    eng.scan(&[fx.watch()], t);
    // the file grows before the settle window elapses: the clock resets
    fs::write(&p, b"part one and more").unwrap();
    eng.scan(&[fx.watch()], t + 5.0);
    eng.step(&srv, t + 5.0);
    assert!(srv.puts.borrow().is_empty(), "a changing file is not uploaded");
    // now it holds still long enough
    eng.scan(&[fx.watch()], t + 25.0);
    eng.step(&srv, t + 25.0);
    assert_eq!(srv.puts.borrow().len(), 1);
    assert_eq!(srv.puts.borrow()[0].2, b"part one and more".len());
}

#[test]
fn work_in_progress_and_hidden_and_os_files_are_skipped() {
    let fx = Fixture::new();
    fx.write("a.crdownload", b"downloading");
    fx.write("~$doc.docx", b"office lock");
    fx.write(".hidden", b"dot");
    fx.write("desktop.ini", b"os");
    fx.write("keep.txt", b"real");
    let srv = FakeServer::default();
    let mut eng = fx.engine();
    let t = now();
    eng.scan(&[fx.watch()], t);
    eng.scan(&[fx.watch()], t + 20.0);
    eng.step(&srv, t + 20.0);
    let names: Vec<String> = srv.puts.borrow().iter().map(|p| p.1.clone()).collect();
    assert_eq!(names, vec!["keep.txt"]);
}

#[test]
fn subfolders_are_not_descended() {
    let fx = Fixture::new();
    fs::create_dir_all(fx.watch().join("sub")).unwrap();
    fs::write(fx.watch().join("sub").join("inner.txt"), b"nested").unwrap();
    fx.write("top.txt", b"top");
    let srv = FakeServer::default();
    let mut eng = fx.engine();
    let t = now();
    eng.scan(&[fx.watch()], t);
    eng.scan(&[fx.watch()], t + 20.0);
    eng.step(&srv, t + 20.0);
    let names: Vec<String> = srv.puts.borrow().iter().map(|p| p.1.clone()).collect();
    assert_eq!(names, vec!["top.txt"], "only top-level files are taken");
}

#[test]
fn a_blob_the_server_already_has_is_not_uploaded_again_but_is_still_recorded() {
    let fx = Fixture::new();
    fx.write("dup.txt", b"seen before");
    let srv = FakeServer::default();
    srv.known.borrow_mut().insert(sha256_hex(b"seen before"));
    let mut eng = fx.engine();
    let t = now();
    eng.scan(&[fx.watch()], t);
    eng.scan(&[fx.watch()], t + 20.0);
    eng.step(&srv, t + 20.0);
    // has() said yes, so no bytes were pushed, but the file is marked done and a sighting was recorded
    assert!(srv.puts.borrow().is_empty(), "known bytes are not re-uploaded");
    assert_eq!(srv.sightings.borrow().len(), 1, "a sighting is still recorded");
    assert!(eng.is_done(&fx.watch().join("dup.txt")), "but it is recorded as kept");
}

#[test]
fn an_interrupted_run_resends_the_same_file_and_never_loses_the_original() {
    let fx = Fixture::new();
    let p = fx.write("order.pdf", b"purchase order");
    let srv = FakeServer::default();
    *srv.fail.borrow_mut() = true;
    let t = now();
    {
        let mut eng = fx.engine();
        eng.scan(&[fx.watch()], t);
        eng.scan(&[fx.watch()], t + 20.0);
        eng.step(&srv, t + 20.0); // upload fails (offline)
    }
    assert!(p.exists(), "the original is never removed");
    assert!(srv.puts.borrow().is_empty());
    // a new process opens the journal and the server is back
    *srv.fail.borrow_mut() = false;
    {
        let mut eng = fx.engine();
        eng.scan(&[fx.watch()], t + 40.0);
        eng.step(&srv, t + 40.0);
    }
    assert_eq!(srv.puts.borrow().len(), 1, "the same file is picked up and sent after recovery");
    assert!(p.exists(), "a kept file is left in place (we store, we do not move)");
}

#[test]
fn the_journal_is_written_so_a_restart_does_not_re_upload_a_kept_file() {
    let fx = Fixture::new();
    fx.write("done.txt", b"already kept");
    let srv = FakeServer::default();
    let t = now();
    {
        let mut eng = fx.engine();
        eng.scan(&[fx.watch()], t);
        eng.scan(&[fx.watch()], t + 20.0);
        eng.step(&srv, t + 20.0);
    }
    assert_eq!(srv.puts.borrow().len(), 1);
    // a restart: the journal remembers the file is kept, so it is not sent again
    {
        let mut eng = fx.engine();
        eng.scan(&[fx.watch()], t + 40.0);
        eng.step(&srv, t + 40.0);
    }
    assert_eq!(srv.puts.borrow().len(), 1, "a kept file is not uploaded again after a restart");
}

#[test]
fn the_journal_file_lands_where_asked() {
    let fx = Fixture::new();
    fx.write("x.txt", b"x");
    let srv = FakeServer::default();
    let t = now();
    let mut eng = fx.engine();
    eng.scan(&[fx.watch()], t);
    eng.scan(&[fx.watch()], t + 20.0);
    eng.step(&srv, t + 20.0);
    assert!(Path::new(&fx.journal()).exists());
}

#[test]
fn an_empty_file_is_skipped_not_uploaded() {
    let fx = Fixture::new();
    fx.write("empty.txt", b"");
    fx.write("z_real.txt", b"has content");
    let srv = FakeServer::default();
    let mut eng = fx.engine();
    let t = now();
    eng.scan(&[fx.watch()], t);
    eng.scan(&[fx.watch()], t + 20.0);
    eng.step(&srv, t + 20.0);
    let names: Vec<String> = srv.puts.borrow().iter().map(|p| p.1.clone()).collect();
    assert_eq!(names, vec!["z_real.txt"], "an empty file is never uploaded, the real one still is");
    assert!(fx.watch().join("empty.txt").exists(), "and the empty original is left in place");
}

#[test]
fn a_file_over_the_cap_is_skipped_without_reading_or_blocking() {
    let fx = Fixture::new();
    fx.write("huge.bin", &vec![b'X'; 2048]);
    fx.write("z_small.txt", b"ok");
    let srv = FakeServer::default();
    // cap at 1 KiB: huge.bin is over it, z_small.txt is under
    let mut eng = IntakeEngine::open_with_settle(&fx.journal(), SETTLE_S).unwrap().with_max_bytes(1024);
    let t = now();
    eng.scan(&[fx.watch()], t);
    eng.scan(&[fx.watch()], t + 20.0);
    eng.step(&srv, t + 20.0);
    let names: Vec<String> = srv.puts.borrow().iter().map(|p| p.1.clone()).collect();
    assert_eq!(names, vec!["z_small.txt"], "the oversize file is never uploaded, the small one still is");
    assert!(fx.watch().join("huge.bin").exists(), "and the oversize original is left in place");
}

#[test]
fn a_file_the_server_refuses_does_not_block_the_files_after_it() {
    let fx = Fixture::new();
    // "a_big" sorts before "b_ok" in the journal's key order, so if a refusal stopped the pass, b_ok would never go
    fx.write("a_big.bin", b"too large for the server");
    fx.write("b_ok.txt", b"fine");
    let srv = FakeServer::default();
    srv.reject.borrow_mut().insert("a_big.bin".to_string());
    let mut eng = fx.engine();
    let t = now();
    eng.scan(&[fx.watch()], t);
    eng.scan(&[fx.watch()], t + 20.0);
    eng.step(&srv, t + 20.0);
    let names: Vec<String> = srv.puts.borrow().iter().map(|p| p.1.clone()).collect();
    assert_eq!(names, vec!["b_ok.txt"], "the refused file is skipped, the good one still uploads");
    // the refused file is remembered and not retried on the next pass
    eng.scan(&[fx.watch()], t + 40.0);
    eng.step(&srv, t + 40.0);
    assert_eq!(srv.puts.borrow().len(), 1, "a refused file is not tried again");
    assert!(fx.watch().join("a_big.bin").exists(), "and its original is left in place");
}
