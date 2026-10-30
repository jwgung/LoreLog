//! The librarian's command line. Ported from the Python server's test_librarian_cli.py.
use std::fs;
use std::process::Command;

fn run(root: &std::path::Path, args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_life-librarian"))
        .args(args)
        .env("LIFE_VAULT", root.join("vault"))
        .env("LIFE_DATA_DIR", root.join("data"))
        .env("LIFE_CONF_DIR", root.join("conf"))
        .env("LIFE_DIARY_CONFIG", root.join("conf/diary.json"))
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

fn root() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let mail = dir.path().join("vault/Mail/2023/2023-08-01-0900-cli.md");
    fs::create_dir_all(mail.parent().unwrap()).unwrap();
    fs::write(mail, "---\nmail_source: account/INBOX\ndate: 2023-08-01\nsubject: \"명령줄 확인\"\n---\n## Message\n명령줄문구도롱뇽\n").unwrap();
    dir
}

#[test]
fn update_search_stats_and_rebuild() {
    let d = root();
    let (code, out, _) = run(d.path(), &["update"]);
    assert_eq!(code, 0);
    assert!(out.starts_with("indexed 1 files"), "{out}");
    assert!(run(d.path(), &["update"]).1.starts_with("indexed 0 files"), "nothing new the second time");
    let (code, out, _) = run(d.path(), &["search", "도롱뇽"]);
    assert_eq!(code, 0);
    assert!(out.contains("2023-08-01") && out.contains("명령줄 확인"));
    assert!(run(d.path(), &["stats"]).1.contains("documents from"));
    let (code, out, _) = run(d.path(), &["rebuild"]);
    assert_eq!(code, 0);
    assert!(!out.starts_with("indexed 0 files"), "a rebuild reads everything again");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let index = d.path().join("data/librarian.sqlite");
        assert_eq!(fs::metadata(&index).unwrap().permissions().mode() & 0o777, 0o600, "the index is private");
    }
}

#[test]
fn nothing_found_and_bad_arguments() {
    let d = root();
    run(d.path(), &["update"]);
    assert!(run(d.path(), &["search", "어디에도없는도롱뇽말"]).1.contains("nothing found"));
    for args in [&[][..], &["search"], &["search", "x", "--limit", "many"], &["nonsense"]] {
        assert_eq!(run(d.path(), args).0, 2, "{args:?}");
    }
}

#[test]
fn an_index_never_built_is_not_searched_and_says_so() {
    let d = root();
    let (code, out, err) = run(d.path(), &["search", "도롱뇽"]);
    assert_eq!(code, 1);
    assert!(!out.contains("명령줄 확인"));
    assert!(err.contains("life-librarian update"));
    assert!(run(d.path(), &["stats"]).2.contains("not built yet"));
    run(d.path(), &["update"]);
    assert!(run(d.path(), &["search", "도롱뇽"]).1.contains("명령줄 확인"));
}
