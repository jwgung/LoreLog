"""Command line of life-ingest, the collection endpoint.

life-ingest: collection endpoint that any client (phone, PC, ...) can send NDJSON to. Raw data is stored on this server.

  POST /ingest/<source>      body: NDJSON (one JSON object per line), header: Authorization: Bearer <token>
  GET  /diary/<date>         the day's draft (summary + question); generated on demand -> 202 while generating
  POST /diary/<date>         body: JSON {corrections:[{wrong,right}], reflection}; stored via life-diary
  GET  /diary/senders        notification senders of the last 14 days with the user's allowed flag
  POST /diary/senders        body: {allow: ["<app>: <sender>" | "<app>: *"]}; which messengers may feed the diary
  GET  /diary/<date>/chat    the day's chat exchange (to show it again)
  POST /diary/<date>/chat    body: {message, wrong?}; stores it in the diary and returns the assistant's reply (local LLM)
  POST /transcribe           body: audio (wav); returns {text} from the local speech model (LIFE_ASR_URL); nothing is stored
  GET  /todos                open and recently done to-dos (+ extracting flag)
  POST /todos                body: {text}; add the user's own to-do
  POST /todos/<id>           body: {action: done|dismiss|reopen}
  POST /todos/extract        extract new to-dos from the records in the background (throttled)
  GET  /healthz              only reports that it is alive (no auth, reveals nothing else)

- Raw data is only ever appended to <data dir>/raw/<source>/<YYYY-MM-DD>.ndjson; nothing is edited or deleted.
  The date comes from the record's ts (epoch milliseconds, LoreLog format) or an ISO time, else from the receive
  time (server local time).
- A line that arrives again (a resend) is stored only once.
- Records are kept as they are; only _src, _recv and _class are added. _class defaults to personal.
- Independent of how clients connect (Tailscale, same network, tunnel): where to listen is the INGEST_BIND setting.

Settings: ~/.config/vault-sync/ingest.env  (KEY=VALUE, mode 600)
  INGEST_TOKEN=...            required. The value clients send as the Bearer token (at least 16 characters)
  INGEST_BIND=127.0.0.1       address to listen on; for Tailscale use this machine's tailnet address
  INGEST_PORT=8765
  LIFE_DATA_DIR=...           default: the OS standard location (Linux ~/.local/share/life, Windows %LOCALAPPDATA%\\life,
                              macOS ~/Library/Application Support/life)
  INGEST_CLASS=personal       default class (work | personal)
  INGEST_MAX_MB=20            maximum size of one request
  INGEST_CERT / INGEST_KEY    HTTPS if both are set

Usage: life-ingest [--selftest] [-h]
"""
import json
import os
import sys
import threading
from datetime import datetime
from pathlib import Path

from life.ingest.http import serve
from life.ingest.settings import ENV_FILE, data_root, load_env


def selftest():
    import tempfile
    import urllib.error
    import urllib.request
    tmp = Path(tempfile.mkdtemp(prefix="life-ingest-test-"))
    srv = serve("127.0.0.1", 0, tmp, "secret-token", "personal", 1024 * 1024)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    def post(path, body, token="secret-token"):
        req = urllib.request.Request(f"http://127.0.0.1:{port}{path}", data=body.encode(), method="POST",
                                     headers={"Authorization": f"Bearer {token}", "Content-Type": "application/x-ndjson"})
        try:
            with urllib.request.urlopen(req) as r:
                return r.status, json.loads(r.read())
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read())

    ts = int(datetime(2026, 10, 26, 13, 5).timestamp() * 1000)
    body = "\n".join([json.dumps({"type": "voice", "ts": ts, "text": "회의 시작"}, ensure_ascii=False),
                      json.dumps({"type": "app", "ts": ts + 1000, "pkg": "com.x"}), "이건 JSON 이 아님", "[1,2]"]) + "\n"
    assert post("/ingest/lorelog", body) == (200, {"accepted": 2, "duplicates": 0, "rejected": 2})
    assert post("/ingest/lorelog", body) == (200, {"accepted": 0, "duplicates": 2, "rejected": 2}), "a resend must be filtered out as duplicates"
    assert post("/ingest/lorelog", body, token="wrong")[0] == 401
    assert post("/ingest/BadSource", body)[0] == 404
    f = tmp / "raw" / "lorelog" / "2026-10-26.ndjson"
    lines = f.read_text(encoding="utf-8").splitlines()
    rec = json.loads(lines[0])
    assert len(lines) == 2 and rec["text"] == "회의 시작" and rec["_src"] == "lorelog" and rec["_class"] == "personal"
    if os.name == "posix":
        assert oct(f.stat().st_mode & 0o777) == "0o600" and oct((tmp / "raw").stat().st_mode & 0o777) == "0o700"
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/healthz") as r:
        assert json.loads(r.read()) == {"ok": True}
    srv.shutdown()
    print("selftest ok:", lines[0][:110])



def main():
    if {"-h", "--help"} & set(sys.argv[1:]):
        print(__doc__.strip())
        return
    if "--selftest" in sys.argv:
        return selftest()
    cfg = load_env()
    token = cfg.get("INGEST_TOKEN", "")
    if len(token) < 16:
        print(f"{ENV_FILE} needs INGEST_TOKEN (at least 16 characters).", file=sys.stderr)
        sys.exit(2)
    root = data_root(cfg)
    bind = cfg.get("INGEST_BIND", "127.0.0.1")
    port = int(cfg.get("INGEST_PORT", "8765"))
    srv = serve(bind, port, root, token, cfg.get("INGEST_CLASS", "personal"), int(float(cfg.get("INGEST_MAX_MB", "20")) * 1024 * 1024),
                cfg.get("INGEST_CERT"), cfg.get("INGEST_KEY"))
    print(f"life-ingest started: {bind}:{port}  data: {root}  {'HTTPS' if cfg.get('INGEST_CERT') else 'HTTP'}", file=sys.stderr)
    sys.stderr.flush()
    try:
        srv.serve_forever()
    except KeyboardInterrupt:
        pass

