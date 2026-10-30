//! life-diary: the day's diary from the command line (what the HTTP API does, for scripts, timers and checks).
use std::io::Read;
use std::process::ExitCode;

use life_server::assistant::{chat, context, diary, senders, todos};
use life_server::config::Paths;
use life_server::llm::Http;
use serde_json::{Value, json};

const HELP: &str = "\
usage: life-diary [date] [options]

Make the day's summary with the local model and write it into the diary note (default), or one of:
date: YYYY-MM-DD (default: today)

  --context        print the context sent to the LLM and exit
  --json           print the stored draft, corrections and reflection of the day as JSON
  --save           read {corrections:[{wrong,right}], reflection} from stdin and store it
  --senders        print notification senders of the last 14 days with their allowed flag (JSON)
  --set-allow      read {allow:['label: title' | 'label: *']} from stdin and store it
  --chatlog        print the day's chat exchange as JSON
  --chat           read {message, wrong?} from stdin; save it and print the assistant's reply as JSON
  --todos          print proposed, open and recently done to-dos as JSON
  --todos-extract  extract new to-dos from the day's records (throttled)
  --todo-act       read {id, action: accept|done|dismiss|reopen} from stdin (accept: proposed only, reopen: done only)
  --todo-add       read {text} from stdin and add it as the user's own to-do
  --dry-run        print the generated draft instead of writing it";

fn stdin_json() -> Result<Value, String> {
    let mut s = String::new();
    std::io::stdin().read_to_string(&mut s).map_err(|e| e.to_string())?;
    serde_json::from_str(&s).map_err(|e| format!("stdin is not JSON: {e}"))
}

fn text_of(v: &Value, key: &str) -> Result<String, String> {
    v.get(key).and_then(Value::as_str).map(str::to_string).ok_or_else(|| format!("stdin needs \"{key}\""))
}

fn run(args: &[String], paths: &Paths, day: &str) -> Result<(), String> {
    let flag = |f: &str| args.iter().any(|a| a == f);
    let model = Http::from_env();
    if flag("--todos") {
        println!("{}", todos::view(paths));
    } else if flag("--todos-extract") {
        let n = todos::extract(paths, &model, day, false).map_err(|e| format!("todo extract failed: {e}"))?;
        println!("{}", json!({"added": n}));
    } else if flag("--todo-act") {
        let v = stdin_json()?;
        match todos::act(paths, &text_of(&v, "id")?, &text_of(&v, "action")?).map_err(|e| e.to_string())? {
            todos::Acted::Changed => {}
            todos::Acted::Missing => return Err("no such to-do or action".into()),
            todos::Acted::NotNow => return Err("the to-do is not in a state for that action".into()),
        }
    } else if flag("--todo-add") {
        todos::add_user(paths, &text_of(&stdin_json()?, "text")?, day).map_err(|e| e.to_string())?;
    } else if flag("--chatlog") {
        println!("{}", json!({"messages": chat::chat_history(paths, day)}));
    } else if flag("--chat") {
        let v = stdin_json()?;
        let wrong = v.get("wrong").and_then(Value::as_str).filter(|w| !w.is_empty());
        let recall =
            |m: &str, d: &str, said: &[String]| life_server::assistant::recall::find(paths, &model, m, d, said);
        let reply = chat::chat_reply(paths, &model, &recall, day, &text_of(&v, "message")?, wrong)
            .map_err(|e| format!("chat failed: {e}"))?;
        println!("{}", json!({"reply": reply}));
    } else if flag("--senders") {
        println!("{}", Value::Array(senders::list_senders(paths, 14)));
    } else if flag("--set-allow") {
        let items = stdin_json()?.get("allow").and_then(Value::as_array).cloned().unwrap_or_default();
        senders::save_allow(paths, &items).map_err(|e| e.to_string())?;
    } else if flag("--json") {
        println!("{}", diary::read_day(paths, day));
    } else if flag("--save") {
        diary::save_user(paths, day, &stdin_json()?).map_err(|e| e.to_string())?;
    } else if flag("--context") {
        println!("{}", context::build_context(paths, day, true));
    } else {
        let dry = flag("--dry-run");
        let text = life_server::assistant::summary::generate(paths, &model, day, dry)
            .map_err(|e| format!("LLM request failed: {e}"))?;
        if dry {
            println!("{text}");
        } else {
            println!("wrote {}", paths.vault.join("Diary").join(format!("{day}.md")).display());
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    life_server::logging::to_stderr();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let day = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .cloned()
        .unwrap_or_else(|| chrono::Local::now().format("%Y-%m-%d").to_string());
    if chrono::NaiveDate::parse_from_str(&day, "%Y-%m-%d").is_err() {
        eprintln!("date must be YYYY-MM-DD: {day}");
        return ExitCode::from(2);
    }
    match run(&args, &Paths::from_env(), &day) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}
