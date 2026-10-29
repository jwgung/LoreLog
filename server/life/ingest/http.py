"""HTTP surface of life-ingest: a route table, Bearer auth and the server factory. The work itself lives in other modules."""
import hmac
import json
import re
import ssl
import sys
from datetime import datetime
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import unquote

from life.assistant.chat import chat_history, chat_reply
from life.assistant.diaryfile import save_user
from life.assistant.senders import list_senders, load_allow, save_allow
from life.assistant.todos import todo_act, todo_add_user
from life.ingest import jobs
from life.ingest.asr import transcribe_audio
from life.ingest.filestore import has_blob, keep_file, record_sighting
from life.ingest.store import store

DAY = r"(\d{4}-\d{2}-\d{2})"
BODY_LIMIT = 1_000_000

class Reject(Exception):
    """Abort the request with this status and body."""

    def __init__(self, code, body):
        super().__init__(code)
        self.code, self.body = code, body


def make_handler(root, token, cls, max_bytes):
    class H(BaseHTTPRequestHandler):
        server_version = "life-ingest"

        def _send(self, code, obj):
            data = json.dumps(obj, ensure_ascii=False).encode("utf-8")
            self.send_response(code)
            self.send_header("Content-Type", "application/json; charset=utf-8")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def log_message(self, fmt, *args):  # never log the body or headers (the token)
            pass

        def _authorized(self):
            given = self.headers.get("Authorization", "")
            return given.startswith("Bearer ") and hmac.compare_digest(given[7:].strip().encode(), token.encode())

        def _length(self):
            try:
                return int(self.headers.get("Content-Length", "0"))
            except ValueError:
                raise Reject(400, {"error": "bad length"})

        def _json(self, limit=BODY_LIMIT):
            try:
                return json.loads(self.rfile.read(min(self._length(), limit)).decode("utf-8"))
            except ValueError:
                raise Reject(400, {"error": "bad body"})

        def _blob(self):
            n = self._length()
            if n <= 0 or n > max_bytes:
                raise Reject(413, {"error": "too large or empty", "max_bytes": max_bytes})
            return self.rfile.read(n)

        # ---- handlers: (match) -> (status, body) ----

        def _todos_post(self, m):
            payload = self._json(100_000)
            text = str(payload.get("text", "")).strip()[:200]
            if not text:
                return 400, {"error": "empty"}
            todo_add_user(text, datetime.now().strftime("%Y-%m-%d"))
            return 200, {"ok": True}

        def _todo_act(self, m):
            action = self._json(100_000).get("action")
            if action not in ("done", "dismiss", "reopen"):
                return 400, {"error": "bad action"}
            return (200, {"ok": True}) if todo_act(m.group(1), action) else (404, {"error": "not found"})

        def _todos_extract(self, m):
            jobs.extract_todos()
            return 202, {"status": "extracting"}

        def _senders_post(self, m):
            save_allow(self._json().get("allow", []))
            return 200, {"saved": True}

        def _chat_post(self, m):
            payload = self._json()
            if not isinstance(payload, dict) or not isinstance(payload.get("message"), str) or not payload["message"].strip():
                return 400, {"error": "bad body"}
            try:
                return 200, {"reply": chat_reply(m.group(1), payload["message"], payload.get("wrong"))}
            except Exception as e:  # noqa: BLE001
                return 502, {"error": f"chat failed: {e}"[:300]}

        def _diary_post(self, m):
            save_user(m.group(1), self._json())
            return 200, {"saved": True}

        def _transcribe(self, m):
            try:
                return 200, {"text": transcribe_audio(self._blob())}
            except Reject:
                raise
            except Exception as e:  # noqa: BLE001
                return 502, {"error": f"asr failed: {type(e).__name__}"}

        def _ingest(self, m):
            body = self._blob().decode("utf-8", errors="replace")
            ok, dup, bad = store(root, m.group(1), body, cls)
            sys.stderr.write(f"{datetime.now():%H:%M:%S} {m.group(1)} stored {ok} duplicates {dup} rejected {bad}\n")
            sys.stderr.flush()
            return 200, {"accepted": ok, "duplicates": dup, "rejected": bad}

        def _file_has(self, m):
            return 200, {"sha256": m.group(1), "present": has_blob(root, m.group(1))}

        def _file_put(self, m):
            sha = (self.headers.get("X-File-Sha256") or "").strip().lower()
            name = unquote(self.headers.get("X-File-Name") or "")[:400] or "(unnamed)"
            mime = (self.headers.get("X-File-Mime") or "application/octet-stream")[:200]
            ts = self.headers.get("X-File-Ts")
            when = int(ts) if ts and ts.isdigit() else int(datetime.now().timestamp() * 1000)
            if (self.headers.get("X-File-Sighting") or "").strip() == "1":
                try:
                    record_sighting(root, sha, name, mime, when, cls)  # size comes from the kept blob
                except KeyError:
                    return 409, {"error": "blob not present"}
                return 200, {"sha256": sha, "stored": False, "duplicate": True}
            data = self._blob()
            try:
                sha, stored, dup = keep_file(root, data, sha, name, mime, when, cls)
            except ValueError:
                return 400, {"error": "hash mismatch"}
            safe = name.encode("unicode_escape").decode("ascii")  # no raw newlines etc. in the journal line
            sys.stderr.write(f"{datetime.now():%H:%M:%S} file {sha[:12]} {'stored' if stored else 'dup'} {safe}\n")
            sys.stderr.flush()
            return 200, {"sha256": sha, "stored": stored, "duplicate": dup}


        # (method, path regex, handler). All but /healthz need the Bearer token. Order matters: /diary/senders before /diary/<date>.
        ROUTES = [
            ("GET", r"/healthz", lambda s, m: (200, {"ok": True})),
            ("GET", r"/todos", lambda s, m: (200, jobs.todos())),
            ("POST", r"/todos", _todos_post),
            ("POST", r"/todos/extract", _todos_extract),
            ("POST", r"/todos/([0-9a-f]{10})", _todo_act),
            ("GET", r"/diary/senders", lambda s, m: (200, list_senders())),
            ("GET", r"/diary/allow", lambda s, m: (200, {"allow": sorted(load_allow())})),
            ("POST", r"/diary/senders", _senders_post),
            ("GET", rf"/diary/{DAY}/chat", lambda s, m: (200, {"messages": chat_history(m.group(1))})),
            ("POST", rf"/diary/{DAY}/chat", _chat_post),
            ("GET", rf"/diary/{DAY}(\?refresh=1)?", lambda s, m: jobs.draft(m.group(1), bool(m.group(2)))),
            ("POST", rf"/diary/{DAY}", _diary_post),
            ("POST", r"/transcribe", _transcribe),
            ("GET", r"/ingest/file/([0-9a-f]{64})", _file_has),
            ("POST", r"/ingest/file", _file_put),
            ("POST", r"/ingest/([a-z0-9_-]{1,32})/?", _ingest),
        ]

        def _dispatch(self, method):
            for meth, pattern, fn in self.ROUTES:
                m = meth == method and re.fullmatch(pattern, self.path)
                if not m:
                    continue
                if self.path != "/healthz" and not self._authorized():
                    return self._send(401, {"error": "unauthorized"})
                try:
                    return self._send(*fn(self, m))
                except Reject as r:
                    return self._send(r.code, r.body)
                except Exception as e:  # noqa: BLE001 - never leak details to the client; the type is enough in the journal
                    sys.stderr.write(f"{datetime.now():%H:%M:%S} {method} {self.path.split('?')[0]} failed: {type(e).__name__}\n")
                    return self._send(500, {"error": "internal error"})
            self._send(404, {"error": "not found"})

        def do_GET(self):
            self._dispatch("GET")

        def do_POST(self):
            self._dispatch("POST")

    return H


def serve(bind, port, root, token, cls, max_bytes, cert=None, key=None):
    srv = ThreadingHTTPServer((bind, port), make_handler(root, token, cls, max_bytes))
    if cert and key:
        ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        ctx.load_cert_chain(cert, key)
        srv.socket = ctx.wrap_socket(srv.socket, server_side=True)
    return srv
