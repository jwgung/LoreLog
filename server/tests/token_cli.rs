//! `life token new`: a new INGEST_TOKEN for the server install, written into ingest.env (kept 0600, the old file kept
//! as ingest.env.bak-token), the next steps printed, the token itself only with --show.
use std::path::Path;
use std::process::{Command, Output};

const OLD: &str = "0123456789abcdef0123456789abcdef";

fn life(conf: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_life"))
        .args(args)
        .env("LIFE_CONF_DIR", conf)
        .env("LIFE_DATA_DIR", conf.join("data"))
        .env("LIFE_VAULT", conf.join("vault"))
        .output()
        .unwrap()
}

fn token_in(conf: &Path) -> String {
    life_server::config::read_env_file(&conf.join("ingest.env")).get("INGEST_TOKEN").cloned().unwrap_or_default()
}

fn install() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ingest.env"), format!("INGEST_BIND=100.101.1.2\nINGEST_TOKEN={OLD}\n")).unwrap();
    dir
}

#[test]
fn token_new_replaces_the_token_keeps_a_backup_and_does_not_print_it() {
    let dir = install();
    let out = life(dir.path(), &["token", "new"]);
    assert!(out.status.success(), "{out:?}");
    let new = token_in(dir.path());
    assert!(new != OLD && new.len() == 32 && new.chars().all(|c| c.is_ascii_hexdigit()), "{new}");
    let text = std::fs::read_to_string(dir.path().join("ingest.env")).unwrap();
    assert!(text.contains("INGEST_BIND=100.101.1.2"), "other settings kept: {text}");
    let bak = std::fs::read_to_string(dir.path().join("ingest.env.bak-token")).unwrap();
    assert!(bak.contains(OLD), "the old file is kept: {bak}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for f in ["ingest.env", "ingest.env.bak-token"] {
            assert_eq!(std::fs::metadata(dir.path().join(f)).unwrap().permissions().mode() & 0o777, 0o600, "{f}");
        }
    }
    let said = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    assert!(!said.contains(&new) && !said.contains(OLD), "never the token without --show: {said}");
    assert!(said.contains("restart life-ingest") && said.contains("--show"), "the next steps: {said}");
    assert!(said.contains("connection code"), "devices reconnect: {said}");
}

#[test]
fn token_new_show_prints_the_new_token() {
    let dir = install();
    let out = life(dir.path(), &["token", "new", "--show"]);
    assert!(out.status.success(), "{out:?}");
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains(&token_in(dir.path())), "{stdout}");
    assert!(!stdout.contains(OLD), "{stdout}");
}

#[test]
fn token_new_without_an_install_or_with_a_wrong_argument_changes_nothing() {
    let empty = tempfile::tempdir().unwrap();
    let out = life(empty.path(), &["token", "new"]);
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    assert!(!empty.path().join("ingest.env").exists(), "no ingest.env is made");
    let dir = install();
    for args in [&["token"][..], &["token", "old"], &["token", "new", "--loud"]] {
        let out = life(dir.path(), args);
        assert_eq!(out.status.code(), Some(2), "{args:?}: {out:?}");
        assert_eq!(token_in(dir.path()), OLD, "{args:?}");
    }
}

#[test]
fn help_explains_token_new() {
    let out = life(Path::new("/nonexistent"), &["--help"]);
    let help = String::from_utf8_lossy(&out.stdout);
    assert!(help.contains("token new [--show]") && help.contains("ingest.env.bak-token"), "{help}");
}
