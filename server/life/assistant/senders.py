"""Which messenger senders may feed the diary (a list the user chooses; default none)."""

import collections
import datetime as dt
import json

from life.assistant.config import ALLOW_FILE, DATA, NOISE_PKGS


def load_allow():
    """Messenger notifications the user allowed for the diary: 'label: title' or 'label: *'. Default: none."""
    try:
        return set(json.loads(ALLOW_FILE.read_text(encoding="utf-8")).get("allow", []))
    except (OSError, ValueError):
        return set()


def save_allow(items):
    ALLOW_FILE.parent.mkdir(parents=True, exist_ok=True)
    tmp = ALLOW_FILE.with_suffix(".tmp")
    tmp.write_text(json.dumps({"allow": sorted(set(items))}, ensure_ascii=False, indent=1), encoding="utf-8")
    tmp.chmod(0o600)
    tmp.replace(ALLOW_FILE)


def is_allowed(allow, label, title):
    return f"{label}: *" in allow or f"{label}: {title}" in allow


def list_senders(days=14):
    """Who sent notifications in the last days: [{label, title, count, allowed}], most active first."""
    allow = load_allow()
    counts = collections.Counter()
    for i in range(days):
        day = (dt.date.today() - dt.timedelta(days=i)).isoformat()
        path = DATA / "raw/lorelog" / f"{day}.ndjson"
        if not path.exists():
            continue
        for line in path.read_text(encoding="utf-8").splitlines():
            try:
                d = json.loads(line)
            except ValueError:
                continue
            if d.get("type") == "notification" and d.get("pkg") not in NOISE_PKGS and d.get("title"):
                counts[(d.get("label"), d.get("title"))] += 1
    return [{"label": l, "title": t, "count": n, "allowed": is_allowed(allow, l, t)} for (l, t), n in counts.most_common()]
