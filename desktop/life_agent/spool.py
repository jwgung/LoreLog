"""Records waiting to be sent: an append-only NDJSON file, so nothing is lost while the server is unreachable."""
import json
import os

from life_agent import config

MAX_BYTES = 20 * 1024 * 1024   # beyond this the oldest lines are dropped rather than filling the disk


def path():
    return config.home() / "spool.ndjson"


def add(records):
    if not records:
        return
    p = path()
    p.parent.mkdir(parents=True, exist_ok=True)
    with open(p, "a", encoding="utf-8") as f:
        for r in records:
            f.write(json.dumps(r, ensure_ascii=False, separators=(",", ":")) + "\n")
    if p.stat().st_size > MAX_BYTES:
        lines = p.read_text(encoding="utf-8").splitlines()
        p.write_text("\n".join(lines[len(lines) // 2:]) + "\n", encoding="utf-8")


def read():
    try:
        return path().read_text(encoding="utf-8").splitlines()
    except OSError:
        return []


def drop(n):
    """Remove the first n lines (they were delivered)."""
    rest = read()[n:]
    p = path()
    if rest:
        tmp = p.with_suffix(".tmp")
        tmp.write_text("\n".join(rest) + "\n", encoding="utf-8")
        os.replace(tmp, p)
    elif p.exists():
        p.unlink()
