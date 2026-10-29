"""Command line of mail-files: keep the attachments of the mails in the configured mail folders (read-only on the server).

Usage: mail-files [--dry-run] [--limit N] [--folder NAME] [-h]

Folders are opened read-only and mails are read with BODY.PEEK, so nothing on the server changes. Every configured
mail folder is used; a folder whose name looks like a sent folder (sent / 보낸 / 발신) marks its files as sent, the
others as received. Processed UIDs are remembered per folder, so a run only reads mails it has not seen. --limit is per
folder. --dry-run reads sizes and headers only and stores nothing.
"""
import email
import email.policy
import imaplib
import json
import re
import sys
from pathlib import Path

from life.collect import mailconf
from life.collect.mailfiles import is_sent_folder, keep
from life.ingest.settings import data_root, load_env

DEFAULT_LIMIT = 100


def state_path(root):
    return root / "mail-files-state.json"


def run_folder(imap, folder, account, root, state, limit, dry):
    """-> (mails read, files stored, bytes written, problem or "")"""
    typ, _ = imap.select(f'"{folder}"', readonly=True)
    if typ != "OK":
        return 0, 0, 0, f"[{folder}] cannot open folder"
    raw_uidv = imap.response("UIDVALIDITY")[1][0]
    uidv = raw_uidv.decode() if raw_uidv else ""
    st = state.get(folder, {})
    if st.get("uidvalidity") != uidv:
        st = {"uidvalidity": uidv, "seen": []}
    seen = set(st["seen"])
    typ, d = imap.uid("search", None, "ALL")
    todo = [u for u in (int(x) for x in d[0].split()) if u not in seen]
    take = todo[:limit]
    print(f"[{folder}] server {len(seen) + len(todo)} · not yet read {len(todo)} (this run {len(take)})")
    if dry:
        size = multipart = 0
        for u in take:
            typ, resp = imap.uid("fetch", str(u), "(RFC822.SIZE BODY.PEEK[HEADER.FIELDS (CONTENT-TYPE)])")
            head = b" ".join(it[0] + it[1] for it in resp if isinstance(it, tuple))
            m = re.search(rb"RFC822\.SIZE (\d+)", head)
            if re.search(rb"multipart/(mixed|related)", head, re.I):
                multipart += 1
                size += int(m.group(1)) if m else 0
        print(f"[dry-run] {multipart} of {len(take)} mails look like they carry attachments, about {size / 1e6:.1f} MB to read")
        return len(take), 0, 0, ""
    mails = files = written = 0
    for u in take:
        typ, resp = imap.uid("fetch", str(u), "(BODY.PEEK[])")
        raw = next((it[1] for it in resp if isinstance(it, tuple)), None)
        seen.add(u)
        if raw is None:
            continue
        msg = email.message_from_bytes(raw, policy=email.policy.default)
        n, w = keep(root, msg, f"{account}/{folder}", "sent" if is_sent_folder(folder) else "received")
        mails, files, written = mails + 1, files + n, written + w
    st["seen"] = sorted(seen)
    state[folder] = st
    return mails, files, written, ""


def main(argv=None):
    args = sys.argv[1:] if argv is None else argv
    if {"-h", "--help"} & set(args):
        print(__doc__.strip())
        return 0
    dry = "--dry-run" in args
    limit = int(args[args.index("--limit") + 1]) if "--limit" in args else DEFAULT_LIMIT
    only = args[args.index("--folder") + 1] if "--folder" in args else None
    cfg = mailconf.load()
    need = [k for k in ("IMAP_HOST", "IMAP_USER", "IMAP_PASSWORD") if not cfg.get(k)]
    if need:
        print(f"Missing {', '.join(need)} ({mailconf.ENV_FILE}, {mailconf.SOURCES_FILE}).", file=sys.stderr)
        return 2
    folders = [only] if only else [f.strip() for f in cfg.get("MAIL_FOLDERS", "INBOX,Sent Messages").split(",") if f.strip()]
    if not folders:
        print("No mail folders configured.", file=sys.stderr)
        return 2
    root = data_root(load_env())
    sp = state_path(root)
    state = json.loads(sp.read_text(encoding="utf-8")) if sp.exists() else {}
    try:
        imap = imaplib.IMAP4_SSL(cfg["IMAP_HOST"], int(cfg.get("IMAP_PORT", "993")))
        imap.login(cfg["IMAP_USER"], cfg["IMAP_PASSWORD"])
    except Exception as e:  # noqa: BLE001 - report only the type so the password cannot leak into the message
        print(f"IMAP connection failed: {type(e).__name__}", file=sys.stderr)
        return 1
    failed, totals = False, [0, 0, 0]
    try:
        for folder in folders:
            mails, files, written, problem = run_folder(imap, folder, cfg.get("MAIL_ACCOUNT", "account"), root, state, limit, dry)
            if problem:
                print(problem, file=sys.stderr)
                failed = True
            totals = [totals[0] + mails, totals[1] + files, totals[2] + written]
            if not dry:
                sp.parent.mkdir(parents=True, exist_ok=True)
                sp.write_text(json.dumps(state), encoding="utf-8")  # after each folder: an interrupted run resumes
    finally:
        try:
            imap.logout()
        except Exception:  # noqa: BLE001
            pass
    if not dry:
        print(f"read {totals[0]} mails · {totals[1]} attachments seen · {totals[2] / 1e6:.1f} MB newly written")
    return 1 if failed else 0
