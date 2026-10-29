"""Keep the files that went through the user's mail, sent and received: the originals, as they were.

A deliverable is what a person makes to share and to leave evidence. Mail notes only list attachment names, so the
file itself is not kept anywhere. This stores each attachment of a mail:

  <data>/raw/files/<sha256[:2]>/<sha256>        the bytes, untouched (the name of the file is its content hash)
  <data>/raw/mailfile/<YYYY-MM-DD>.ndjson       one line per attachment: where and when it was seen (raw store format)

Nothing is converted, summarised or tagged, and who it went to or came from is not copied: that is in the mail note
and can be worked out later. Each line says only whether the mail was sent or received. The same file seen again (a resend, a forward) is one blob and a line per sighting. The content
hash is what lets another channel (a window title, a messenger, a print job) recognise the same deliverable later.
"""
import email
import email.policy
import email.utils
import hashlib
import json
import os
import re
from datetime import datetime
from pathlib import Path

from life.ingest.store import ensure_dir, store

SENT_NAME_RE = re.compile(r"sent|보낸|발신", re.I)


def is_sent_folder(name):
    return bool(SENT_NAME_RE.search(name))


def norm_id(v):
    return (v or "").strip().strip("<>").strip()


def attachments(msg):
    """[(file name, mime type, bytes)] of the attachments of a parsed mail (a part with no name gets a placeholder)."""
    out = []
    for part in msg.iter_attachments():
        data = part.get_content() if part.get_content_maintype() != "message" else part.as_bytes()
        if isinstance(data, str):
            data = data.encode(part.get_content_charset() or "utf-8", errors="replace")
        out.append((part.get_filename() or "(unnamed)", part.get_content_type(), bytes(data)))
    return out


def blob_path(root, sha):
    return root / "raw" / "files" / sha[:2] / sha


def put_blob(root, data):
    """Write the bytes once, under their hash. -> (sha256, newly written)"""
    sha = hashlib.sha256(data).hexdigest()
    path = blob_path(root, sha)
    if path.exists() and path.stat().st_size == len(data):
        return sha, False
    ensure_dir(root)
    ensure_dir(root / "raw")
    ensure_dir(root / "raw" / "files")
    ensure_dir(path.parent)
    tmp = path.with_suffix(".tmp")
    tmp.write_bytes(data)
    os.chmod(tmp, 0o600)
    os.replace(tmp, path)
    return sha, True


def keep(root, msg, source, direction="sent", cls="personal"):
    """Store the attachments of one mail (direction: "sent" or "received"). -> (files stored, bytes newly written)"""
    files = attachments(msg)
    if not files:
        return 0, 0
    mail_id = norm_id(msg.get("Message-ID"))
    try:
        when = email.utils.parsedate_to_datetime(str(msg.get("Date"))).astimezone()
    except Exception:  # noqa: BLE001 - a mail with no usable date is still kept, dated now
        when = datetime.now().astimezone()
    lines, written = [], 0
    for name, mime, data in files:
        sha, new = put_blob(root, data)
        written += len(data) if new else 0
        lines.append(json.dumps({"type": "file", "ts": int(when.timestamp() * 1000), "sha256": sha, "name": name, "mime": mime,
                                 "size": len(data), "mail_id": mail_id, "mail_source": source, "direction": direction}, ensure_ascii=False))
    store(root, "mailfile", "\n".join(lines) + "\n", cls)
    return len(files), written
