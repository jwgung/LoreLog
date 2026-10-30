//! mail-files: keep the attachments of the mails in the configured mail folders (read-only on the server).
use std::process::ExitCode;

use life_server::collect::settings;
use life_server::collect::sync::{self, MailRun};
use life_server::config::Paths;

const HELP: &str = "\
Command line of mail-files: keep the attachments of the mails in the configured mail folders (read-only on the server).

Usage: mail-files [--dry-run] [--limit N] [--folder NAME] [-h]

Folders are opened read-only and mails are read with BODY.PEEK, so nothing on the server changes. Every configured
mail folder is used (--folder NAME: only that one); a folder whose name looks like a sent folder (sent / 보낸 / 발신)
marks its files as sent, the others as received. Processed UIDs are remembered per folder, so a run only reads mails
it has not seen. --limit is per folder. --dry-run reads sizes and headers only and stores nothing.";

fn main() -> ExitCode {
    life_server::logging::to_stderr();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let run = MailRun {
        dry: args.iter().any(|a| a == "--dry-run"),
        limit: settings::arg(&args, "--limit").and_then(|v| v.parse().ok()).unwrap_or(sync::FILES_LIMIT),
        folder: settings::arg(&args, "--folder"),
    };
    ExitCode::from(sync::files(&Paths::from_env(), &run, &mut sync::console).exit_code())
}
