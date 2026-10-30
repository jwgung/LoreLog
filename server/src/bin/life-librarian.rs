//! life-librarian (사서): keep the index of every record and search it.
use std::process::ExitCode;
use std::time::Duration;

use life_server::collect::sync;
use life_server::config::Paths;
use life_server::librarian::{Librarian, Outdated};
use serde_json::Value;

const HELP: &str = "\
usage: life-librarian {update,rebuild,search,stats} ...

Command line of the librarian (사서): keep the index of every record and search it.

  update                 index new and changed records, forget removed ones (life sync runs this)
  rebuild                throw the index away and build it again from the records
  search WORDS... [--limit N]
                         what the librarian finds for these words, across all days
  stats                  how much is indexed

The index is a copy at <data>/librarian.sqlite (mode 600). Only records the diary rules allow are indexed:
messengers the user allowed, voice in conversations the diary counts as the user's. Deleting the file loses nothing.";

const OUTDATED: &str =
    "the index is from another version or not built yet, and is not searched: run `life-librarian update` first";

fn usage(msg: &str) -> ExitCode {
    eprintln!("usage: life-librarian {{update,rebuild,search,stats}} ...\nlife-librarian: error: {msg}");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() {
        return usage("the following arguments are required: cmd");
    }
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let cmd = args[0].as_str();
    let mut words = Vec::new();
    let mut limit = 10usize;
    let mut rest = args[1..].iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--limit" if cmd == "search" => match rest.next().map(|v| v.parse::<usize>()) {
                Some(Ok(n)) => limit = n,
                _ => return usage("argument --limit: invalid int value"),
            },
            _ if cmd == "search" => words.push(a.clone()),
            other => return usage(&format!("unrecognized arguments: {other}")),
        }
    }
    if !matches!(cmd, "update" | "rebuild" | "search" | "stats") {
        return usage(&format!("argument cmd: invalid choice: '{cmd}'"));
    }
    if cmd == "search" && words.is_empty() {
        return usage("the following arguments are required: words");
    }
    let paths = Paths::from_env();
    if matches!(cmd, "update" | "rebuild") {
        return ExitCode::from(sync::librarian(&paths, cmd == "rebuild", &mut sync::console).exit_code());
    }
    let mut lib = match Librarian::open(&paths, None, Duration::from_secs(30)) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("life-librarian: {e}");
            return ExitCode::FAILURE;
        }
    };
    let result: anyhow::Result<ExitCode> = (|| {
        match cmd {
            "search" => match lib.search(&words.join(" "), limit, None) {
                Err(e) if e.downcast_ref::<Outdated>().is_some() => {
                    eprintln!("{OUTDATED}");
                    return Ok(ExitCode::from(1));
                }
                Err(e) => return Err(e),
                Ok(hits) if hits.is_empty() => println!("nothing found"),
                Ok(hits) => {
                    for h in hits {
                        let s = |k: &str| h[k].as_str().unwrap_or("").to_string();
                        let count = h["count"].as_i64().unwrap_or(1);
                        let span = if count > 1 {
                            format!("  (thread of {count}, {}~{})", s("first"), s("last"))
                        } else {
                            String::new()
                        };
                        println!(
                            "{}  [{}] {}{span}\n    {}\n    {}",
                            s("day"),
                            s("kind"),
                            s("title"),
                            s("snippet"),
                            s("ref")
                        );
                    }
                }
            },
            _ => {
                let st = lib.stats();
                let by: Vec<String> = st["by_source"]
                    .as_object()
                    .map(|o| o.iter().map(|(k, v)| format!("{k} {v}")).collect())
                    .unwrap_or_default();
                println!("{} documents from {} files: {}", st["docs"], st["files"], by.join(", "));
                if st["outdated"] == Value::Bool(true) {
                    eprintln!("{OUTDATED}");
                }
            }
        }
        Ok(ExitCode::SUCCESS)
    })();
    result.unwrap_or_else(|e| {
        eprintln!("life-librarian: {e}");
        ExitCode::FAILURE
    })
}
