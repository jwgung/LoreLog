"""Sends the spooled lines to the server. A line is removed from the spool only after the server accepted its batch."""
import json
import urllib.error
import urllib.request

from life_agent import spool

BATCH = 500


def send_spool(cfg, timeout=15):
    """-> (sent line count, error text or ""). Stops at the first failure and keeps the rest."""
    if not cfg["url"] or not cfg["token"]:
        return 0, "url and token are not set in the config file"
    sent = 0
    while True:
        lines = spool.read()[:BATCH]
        if not lines:
            return sent, ""
        req = urllib.request.Request(
            f"{cfg['url'].rstrip('/')}/ingest/{cfg['source']}", ("\n".join(lines) + "\n").encode("utf-8"), method="POST",
            headers={"Authorization": f"Bearer {cfg['token']}", "Content-Type": "application/x-ndjson"})
        try:
            with urllib.request.urlopen(req, timeout=timeout) as r:
                json.loads(r.read())
        except urllib.error.HTTPError as e:
            return sent, "the server refused the token" if e.code == 401 else f"server answered {e.code}"
        except (urllib.error.URLError, OSError, ValueError) as e:
            return sent, f"cannot reach the server ({type(e).__name__})"
        spool.drop(len(lines))
        sent += len(lines)
