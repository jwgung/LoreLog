"""Raw store: NDJSON lines appended to <data dir>/raw/<source>/<YYYY-MM-DD>.ndjson, never edited or deleted."""
import hashlib
import json
import os
import tempfile
import threading
from datetime import datetime

_lock = threading.Lock()
_hash_cache = {}  # file path -> set of hashes of the lines already stored


def record_day(rec, now):
    for k in ("ts", "timestamp", "time", "t"):
        v = rec.get(k) if isinstance(rec, dict) else None
        if isinstance(v, (int, float)) and v > 0:
            return datetime.fromtimestamp(v / 1000 if v > 1e11 else v)
        if isinstance(v, str):
            try:
                return datetime.fromisoformat(v.replace("Z", "+00:00")).astimezone().replace(tzinfo=None)
            except ValueError:
                pass
    return now


def ensure_dir(p):
    p.mkdir(parents=True, exist_ok=True)
    try:
        os.chmod(p, 0o700)
    except OSError:
        pass  # on Windows etc. folder permissions are left to the OS (ACL)


def blob_path(root, sha):
    """Where a deliverable's bytes live: named by their own content hash, shared by every channel that sees the file."""
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
    # a unique temp name so two uploads of the same content do not share (and corrupt) one temp file
    fd, tmp = tempfile.mkstemp(dir=path.parent, suffix=".tmp")
    try:
        with os.fdopen(fd, "wb") as fh:
            fh.write(data)
        os.chmod(tmp, 0o600)
        os.replace(tmp, path)
    except BaseException:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise
    return sha, True


def store(root, source, body, cls):
    """Append the NDJSON body to the raw files -> (stored, duplicates, rejected)"""
    now = datetime.now()
    recv = now.astimezone().isoformat(timespec="seconds")
    ok = dup = bad = 0
    by_file = {}
    for raw in body.splitlines():
        raw = raw.strip()
        if not raw:
            continue
        try:
            rec = json.loads(raw)
        except ValueError:
            bad += 1
            continue
        if not isinstance(rec, dict):
            bad += 1
            continue
        digest = hashlib.sha1(raw.encode("utf-8")).hexdigest()
        day = record_day(rec, now).strftime("%Y-%m-%d")
        rec.update({"_src": source, "_recv": recv, "_class": cls})
        by_file.setdefault(day, []).append((digest, json.dumps(rec, ensure_ascii=False, separators=(",", ":"))))
    with _lock:
        for day, items in by_file.items():
            d = root / "raw" / source
            ensure_dir(root)
            ensure_dir(root / "raw")
            ensure_dir(d)
            f = d / f"{day}.ndjson"
            seen = _hash_cache.get(f)
            if seen is None:
                seen = set()
                if f.exists():
                    for line in f.read_text(encoding="utf-8", errors="replace").splitlines():
                        try:
                            o = json.loads(line)
                        except ValueError:
                            continue
                        o.pop("_recv", None); o.pop("_src", None); o.pop("_class", None)
                        seen.add(hashlib.sha1(json.dumps(o, ensure_ascii=False).encode("utf-8")).hexdigest())
                _hash_cache[f] = seen
            fresh = []
            for digest, line in items:
                if digest in seen:
                    dup += 1
                else:
                    seen.add(digest); fresh.append(line); ok += 1
            if fresh:
                new_file = not f.exists()
                with open(f, "a", encoding="utf-8") as fh:
                    fh.write("\n".join(fresh) + "\n")
                if new_file:
                    try:
                        os.chmod(f, 0o600)
                    except OSError:
                        pass
    return ok, dup, bad
