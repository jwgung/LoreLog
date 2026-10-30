//! mail-sync: import IMAP mail (inbox and sent folders) into the vault as Mail/ notes. Read-only mirror.
use std::process::ExitCode;

use life_server::collect::settings;
use life_server::collect::sync::{self, MailRun};
use life_server::config::Paths;

const HELP: &str = "\
Import IMAP mail (inbox and sent folders) into the vault as Mail/ notes. Read-only mirror.

- Never modifies the server: folders are opened read-only and bodies are read with BODY.PEEK,
  so read flags do not change.
- Incremental: processed UIDs are recorded per folder (start over if UIDVALIDITY changes). A mail that
  already has a note is skipped, matched by mail_id (Message-ID) or by a date/time/sender/subject fingerprint.
- Notes use the same frontmatter as the existing Mail/ notes created by Mailstone, so hub-sync reads them unchanged.
- Raw .eml files and attachments are not stored (attachments are listed by name only).

Settings live in the OS config place + vault-sync/ (Linux ~/.config/vault-sync/, or LIFE_CONF_DIR):
  env           secrets (KEY=VALUE, mode 600): IMAP_USER, IMAP_PASSWORD
  sources.toml  what to collect, under [mail]: host, port, account, folders, since
                (env may also carry IMAP_HOST, IMAP_PORT, MAIL_ACCOUNT, MAIL_FOLDERS, MAIL_SINCE; sources.toml wins)
Prefer `life add mail` and `life sync` over editing these by hand.

Exit status: 0 on success, 1 if the connection failed or a folder could not be read, 2 if settings are missing.

Usage: mail-sync [--dry-run] [--limit N] [--folder NAME] [-h]";

fn main() -> ExitCode {
    life_server::logging::to_stderr();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{HELP}");
        return ExitCode::SUCCESS;
    }
    let run = MailRun {
        dry: args.iter().any(|a| a == "--dry-run"),
        limit: settings::arg(&args, "--limit").and_then(|v| v.parse().ok()).unwrap_or(sync::MAIL_LIMIT),
        folder: settings::arg(&args, "--folder"),
    };
    ExitCode::from(sync::mail(&Paths::from_env(), &run, &mut sync::console).exit_code())
}
