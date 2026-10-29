"""Keep a PC deliverable's bytes as they are, and record each sighting — the file counterpart of the record store.

A deliverable is what a person makes to share and to leave evidence. The PC agent watches a folder the user chose and
sends each file here. Like mail-files, the bytes go under their own content hash (so the same file seen through mail,
a window title or a print job is one blob) and a line records where and when it was seen. Nothing is converted,
summarised or tagged.

  <data>/raw/files/<sha256[:2]>/<sha256>       the bytes, untouched (shared with mail-files)
  <data>/raw/pcfile/<YYYY-MM-DD>.ndjson        one line per sighting (raw store format)

The agent asks `has_blob` first and only uploads bytes we do not already keep; a resend is one blob and a new line.
"""
import hashlib
import json
import re

from life.ingest.store import blob_path, put_blob, store

SHA256_RE = re.compile(r"[0-9a-f]{64}")


def valid_sha(sha):
    """A sha must be 64 lowercase hex before it is used in a file path (no empty value, no path segments)."""
    return bool(sha) and bool(SHA256_RE.fullmatch(sha))


def has_blob(root, sha):
    """Is this content hash already kept? Lets the agent skip uploading bytes we have."""
    if not valid_sha(sha):
        return False
    p = blob_path(root, sha)
    return p.exists() and p.stat().st_size > 0


def keep_file(root, data, sha, name, mime, when_ms, cls="personal"):
    """Store one PC file: verify the claimed hash, keep the bytes once, record a sighting. -> (sha, stored, duplicate).

    Raises ValueError if the body does not hash to the claimed sha (a corrupt or mislabelled upload)."""
    actual = hashlib.sha256(data).hexdigest()
    if sha and sha != actual:
        raise ValueError("hash mismatch")
    sha = actual
    _, new = put_blob(root, data)
    _record(root, sha, name, mime, len(data), when_ms, cls)
    return sha, new, not new


def record_sighting(root, sha, name, mime, when_ms, cls="personal"):
    """Record that an already-kept file was seen, without the bytes. The size comes from the kept blob, not the
    caller, so it is always right. Raises KeyError if that blob is not kept yet (or the sha is malformed)."""
    if not has_blob(root, sha):
        raise KeyError(sha)
    size = blob_path(root, sha).stat().st_size
    _record(root, sha, name, mime, size, when_ms, cls)


def _record(root, sha, name, mime, size, when_ms, cls):
    rec = {"type": "file", "ts": int(when_ms), "sha256": sha, "name": name, "mime": mime, "size": int(size), "source": "pc"}
    store(root, "pcfile", json.dumps(rec, ensure_ascii=False) + "\n", cls)
