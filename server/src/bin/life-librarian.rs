//! life-librarian (사서): keep the index of every record and search it.
use std::process::ExitCode;
use std::time::Duration;

use life_server::collect::sync;
use life_server::config::Paths;
use life_server::host::Models;
use life_server::librarian::eval::{self, Kind};
use life_server::librarian::{Librarian, Outdated};
use life_server::llm::{Http, HttpSpeech, Model};
use serde_json::Value;

const HELP: &str = "\
usage: life-librarian {update,rebuild,search,stats,eval,graph} ...

Command line of the librarian (사서): keep the index of every record and search it.

  update                 index new and changed records, forget removed ones (life sync runs this)
  rebuild                throw the index away and build it again from the records
  search WORDS... [--limit N]
                         what the librarian finds for these words, across all days
  stats                  how much is indexed
  graph [--limit N]      the model reads up to N records not read yet (default 300; life sync does this each run) and
                         puts what they say into the graph: who and what they are about, roles, states, documents
  eval FILE [--k N] [--no-model]
                         how well the librarian finds what the questions in FILE are about (recall@N, default 8),
                         with the words as asked and with the words the chat model picks (--no-model: only the
                         first). FILE holds real names and subjects: keep it outside the repository (e.g.
                         <data>/eval/recall.toml). Searches the index as it is; nothing is written.

The index is a copy at <data>/librarian.sqlite (mode 600). Only records the diary rules allow are indexed:
messengers the user allowed, voice in conversations the diary counts as the user's. Deleting the file loses nothing.";

const OUTDATED: &str =
    "the index is from another version or not built yet, and is not searched: run `life-librarian update` first";

fn usage(msg: &str) -> ExitCode {
    eprintln!("usage: life-librarian {{update,rebuild,search,stats,eval,graph}} ...\nlife-librarian: error: {msg}");
    ExitCode::from(2)
}

fn main() -> ExitCode {
    life_server::logging::to_stderr();
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
    let (mut k, mut no_model) = (8usize, false);
    let mut rest = args[1..].iter();
    while let Some(a) = rest.next() {
        match a.as_str() {
            "--limit" if cmd == "search" || cmd == "graph" => match rest.next().map(|v| v.parse::<usize>()) {
                Some(Ok(n)) => limit = n,
                _ => return usage("argument --limit: invalid int value"),
            },
            "--k" if cmd == "eval" => match rest.next().map(|v| v.parse::<usize>()) {
                Some(Ok(n)) if n > 0 => k = n,
                _ => return usage("argument --k: invalid int value"),
            },
            "--no-model" if cmd == "eval" => no_model = true,
            _ if cmd == "search" || (cmd == "eval" && words.is_empty()) => words.push(a.clone()),
            other => return usage(&format!("unrecognized arguments: {other}")),
        }
    }
    if !matches!(cmd, "update" | "rebuild" | "search" | "stats" | "eval" | "graph") {
        return usage(&format!("argument cmd: invalid choice: '{cmd}'"));
    }
    if cmd == "search" && words.is_empty() {
        return usage("the following arguments are required: words");
    }
    if cmd == "eval" && words.is_empty() {
        return usage("the following arguments are required: FILE");
    }
    let paths = Paths::from_env();
    if cmd == "eval" && !life_server::librarian::index::default_path(&paths).exists() {
        eprintln!("{OUTDATED}"); // measuring writes nothing, so it does not make an index either
        return ExitCode::FAILURE;
    }
    if cmd == "graph" {
        let n = if args.iter().any(|a| a == "--limit") { limit } else { sync::GRAPH_LIMIT };
        return ExitCode::from(sync::graph(&paths, n, &mut sync::console).exit_code());
    }
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
    if cmd == "eval" {
        return measure(&mut lib, &paths, std::path::Path::new(&words[0]), k, no_model);
    }
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

/// `eval`: every case on its own line, then recall@k per kind. Exit 1 when the file cannot be used.
fn measure(lib: &mut Librarian, paths: &Paths, file: &std::path::Path, k: usize, no_model: bool) -> ExitCode {
    let cases = match eval::load(file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("life-librarian: {}: {e}", file.display());
            return ExitCode::FAILURE;
        }
    };
    // the chat model the server uses (ingest.env or the environment, else the home server's defaults)
    let model = (!no_model).then(|| {
        let (chat, asr) = (Http::from_env(), HttpSpeech::from_env());
        let defaults = Models { chat_url: chat.url, chat_model: chat.model, asr_url: asr.url, asr_model: asr.model };
        let m = Models::load(paths).or(&defaults);
        Http { url: m.chat_url, model: m.chat_model }
    });
    let results = match eval::run(lib, model.as_ref().map(|m| m as &dyn Model), &paths.vault, &cases, k) {
        Ok(r) => r,
        Err(e) if e.downcast_ref::<Outdated>().is_some() => {
            eprintln!("{OUTDATED}");
            return ExitCode::FAILURE;
        }
        Err(e) => {
            eprintln!("life-librarian: {e}");
            return ExitCode::FAILURE;
        }
    };
    let mark = |r: Option<usize>| r.map(|n| format!("O{n:>2}")).unwrap_or_else(|| "X -".into());
    for r in &results {
        let model = match &r.model_words {
            None => String::new(),
            Some(w) if w.is_empty() => "   모델: 찾지 않음".into(),
            Some(w) if r.kind == Kind::Pending => format!("   모델 {w:?}"),
            Some(w) => format!("   모델 {} {:?}", mark(r.model_rank), w),
        };
        let share = r.person_share.map(|(a, b)| format!("  (그 사람과 {a}/{b})")).unwrap_or_default();
        let raw = if r.kind == Kind::Pending { "  · ".into() } else { mark(r.raw_rank) };
        println!("{raw}  {}{share}{model}", r.q);
    }
    println!();
    for (kind, s) in eval::summary(&results) {
        let pct = |(a, b): (usize, usize)| {
            (a * 100).checked_div(b).map(|p| format!("{a}/{b} ({p}%)")).unwrap_or_else(|| "-".into())
        };
        let model =
            if s.model.1 > 0 { format!(", 모델이 고른 검색어 {}", pct(s.model)) } else { String::new() };
        println!("재현율@{k} {}: 말 그대로 {}{model}", kind.label(), pct(s.raw));
    }
    ExitCode::SUCCESS
}
