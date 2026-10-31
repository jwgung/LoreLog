//! A sync run in-process: the jobs a sync runs, what it records for `life status`, and what a dry run leaves alone.
//! No mail or calendar server is reached: the sources here are configured without credentials.
use std::fs;

use life_server::collect::sync::{self, Job, Line, NoSources, Outcome};
use life_server::config::Paths;
use serde_json::Value;

fn setup(sources: &str) -> (tempfile::TempDir, Paths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    fs::create_dir_all(&paths.conf).unwrap();
    fs::write(paths.conf.join("sources.toml"), sources).unwrap();
    (dir, paths)
}

/// Each job as `sync` handed it over: the job, how it ended, what it said.
type Seen = Vec<(Job, Outcome, Vec<Line>)>;

fn run(paths: &Paths, dry: bool) -> (Result<bool, NoSources>, Seen) {
    let mut seen = Vec::new();
    let r = sync::sync(paths, dry, &mut |job, outcome, lines| seen.push((job, outcome, lines.to_vec())));
    (r, seen)
}

#[test]
fn nothing_configured_is_not_a_sync() {
    let (_d, paths) = setup("");
    assert_eq!(run(&paths, false).0, Err(NoSources));
}

#[test]
fn a_job_that_is_not_set_up_is_recorded_as_failed_and_the_librarian_still_runs() {
    let (_d, paths) = setup("[calendar]\nurls = [\"https://cal.example/dav/\"]\n");
    let (r, seen) = run(&paths, false);
    assert_eq!(r, Ok(false));
    let jobs: Vec<(Job, Outcome)> = seen.iter().map(|(j, o, _)| (*j, *o)).collect();
    assert_eq!(jobs, [(Job::Calendar, Outcome::NotSetUp), (Job::Librarian, Outcome::Ok), (Job::Graph, Outcome::Ok)]);
    assert!(matches!(&seen[0].2[..], [Line::Problem(p)] if p.starts_with("Missing calendar URL or credentials")));

    let st: Value = serde_json::from_str(&fs::read_to_string(sync::status_file(&paths)).unwrap()).unwrap();
    assert_eq!(st["calendar"]["ok"], false);
    assert!(st["calendar"]["detail"].as_str().unwrap().starts_with("Missing calendar URL or credentials"));
    assert!(st["calendar"].get("last_ok").is_none());
    assert_eq!(st["librarian"]["ok"], true);
    assert!(st["librarian"]["detail"].as_str().unwrap().starts_with("indexed 0 files"), "the last line it said");
}

#[test]
fn mail_runs_both_mail_jobs_and_a_dry_run_records_nothing() {
    let (_d, paths) = setup("[mail]\nhost = \"imap.example\"\nfolders = [\"INBOX\"]\n");
    let (r, seen) = run(&paths, true);
    assert_eq!(r, Ok(false));
    let jobs: Vec<(Job, Outcome)> = seen.iter().map(|(j, o, _)| (*j, *o)).collect();
    assert_eq!(jobs, [(Job::Mail, Outcome::NotSetUp), (Job::Files, Outcome::NotSetUp)], "no librarian in a dry run");
    assert!(!sync::status_file(&paths).exists());
}

#[test]
fn the_detail_is_the_last_word_of_a_job() {
    let say = |s: &str| Line::Say(s.into());
    let problem = |s: &str| Line::Problem(s.into());
    assert_eq!(sync::detail(Outcome::Ok, &[say("[INBOX] 3"), say("3 new notes")]), "3 new notes");
    assert_eq!(sync::detail(Outcome::Ok, &[say("done"), problem("one file skipped")]), "done", "ok: what it said");
    assert_eq!(sync::detail(Outcome::Failed, &[problem("cannot open"), say("0 new notes")]), "cannot open");
    assert_eq!(sync::detail(Outcome::Failed, &[say("0 new notes")]), "0 new notes");
    assert_eq!(sync::detail(Outcome::Failed, &[]), "");
}
