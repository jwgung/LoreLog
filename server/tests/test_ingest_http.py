import json
import threading
import unittest
import urllib.error
import urllib.request

import _env  # noqa: F401
from life.assistant import todos
from life.ingest import jobs
from life.ingest.http import serve

TOKEN = "test-token-0123456789"


class Http(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.srv = serve("127.0.0.1", 0, _env.ROOT / "ingest", TOKEN, "personal", 1024 * 1024)
        cls.port = cls.srv.server_address[1]
        threading.Thread(target=cls.srv.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()

    def call(self, method, path, body=None, token=TOKEN):
        data = body if isinstance(body, bytes) else (json.dumps(body).encode() if body is not None else None)
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}{path}", data=data, method=method,
                                     headers={"Authorization": f"Bearer {token}"} if token else {})
        try:
            with urllib.request.urlopen(req) as r:
                return r.status, json.loads(r.read())
        except urllib.error.HTTPError as e:
            return e.code, json.loads(e.read())

    def test_everything_but_healthz_needs_the_token(self):
        self.assertEqual(self.call("GET", "/healthz", token=None), (200, {"ok": True}))
        for method, path, body in (("GET", "/todos", None), ("GET", "/diary/senders", None), ("GET", "/diary/2026-01-01", None),
                                   ("POST", "/todos", {}), ("POST", "/transcribe", b"x"), ("POST", "/ingest/lorelog", b"{}")):
            self.assertEqual(self.call(method, path, body, token="wrong")[0], 401, path)

    def test_unknown_paths_are_404(self):
        self.assertEqual(self.call("GET", "/nope")[0], 404)
        self.assertEqual(self.call("POST", "/ingest/BadSource", b"{}")[0], 404)

    def test_todo_add_dismiss_roundtrip(self):
        self.assertEqual(self.call("POST", "/todos", {"text": "  "})[0], 400)
        self.assertEqual(self.call("POST", "/todos", {"text": "A 님께 견적 보내기"}), (200, {"ok": True}))
        _, view = self.call("GET", "/todos")
        item = next(i for i in view["open"] if i["title"] == "A 님께 견적 보내기")
        self.assertFalse(view["extracting"])
        self.assertEqual(self.call("POST", f"/todos/{item['id']}", {"action": "bogus"})[0], 400)
        self.assertEqual(self.call("POST", f"/todos/{item['id']}", {"action": "done"}), (200, {"ok": True}))
        self.assertEqual(self.call("POST", "/todos/0000000000", {"action": "done"})[0], 404)
        _, view = self.call("GET", "/todos")
        self.assertIn("A 님께 견적 보내기", [i["title"] for i in view["done"]])

    def test_senders_allow_roundtrip(self):
        self.assertEqual(self.call("POST", "/diary/senders", {"allow": ["카카오톡: 홍길동"]}), (200, {"saved": True}))
        self.assertEqual(self.call("POST", "/diary/senders", b"not json")[0], 400)

    def test_diary_save_and_chat_log(self):
        self.assertEqual(self.call("POST", "/diary/2026-02-02", {"reflection": "좋은 하루"}), (200, {"saved": True}))
        self.assertEqual(self.call("GET", "/diary/2026-02-02/chat"), (200, {"messages": []}))
        self.assertEqual(self.call("POST", "/diary/2026-02-02/chat", {"message": "   "})[0], 400)

    def test_ingest_stores_once(self):
        line = json.dumps({"type": "voice", "ts": 1792987500000, "text": "안녕"}, ensure_ascii=False).encode() + b"\n"
        self.assertEqual(self.call("POST", "/ingest/lorelog", line), (200, {"accepted": 1, "duplicates": 0, "rejected": 0}))
        self.assertEqual(self.call("POST", "/ingest/lorelog", line), (200, {"accepted": 0, "duplicates": 1, "rejected": 0}))

    def test_draft_failure_is_reported_once_then_retry_is_allowed(self):
        jobs._gen["2026-03-03"] = "LLM request failed: down"
        self.assertEqual(self.call("GET", "/diary/2026-03-03"), (502, {"error": "LLM request failed: down"}))
        self.assertNotIn("2026-03-03", jobs._gen)

    def test_missing_draft_is_generated_in_the_background(self):
        done = threading.Event()
        real = jobs.generate
        jobs.generate = lambda day: done.set()
        try:
            self.assertEqual(self.call("GET", "/diary/2026-04-04"), (202, {"status": "generating"}))
            self.assertTrue(done.wait(5))
        finally:
            jobs.generate = real
        for _ in range(50):  # the thread clears its state right after generate returns
            if "2026-04-04" not in jobs._gen:
                break
            threading.Event().wait(0.05)
        self.assertNotIn("2026-04-04", jobs._gen)


if __name__ == "__main__":
    unittest.main()
