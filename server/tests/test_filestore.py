"""POST /ingest/file keeps a PC deliverable's bytes under its content hash and records a sighting, like mail-files."""
import hashlib
import json
import threading
import unittest
import urllib.error
import urllib.parse
import urllib.request

import _env  # noqa: F401
from life.ingest.http import serve

TOKEN = "test-token-0123456789"


class FileIngest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.root = _env.ROOT / "filestore"
        cls.srv = serve("127.0.0.1", 0, cls.root, TOKEN, "personal", 1024 * 1024)
        cls.port = cls.srv.server_address[1]
        threading.Thread(target=cls.srv.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()

    def req(self, method, path, body=None, headers=None, token=TOKEN):
        h = dict(headers or {})
        if token:
            h["Authorization"] = f"Bearer {token}"
        req = urllib.request.Request(f"http://127.0.0.1:{self.port}{path}", data=body, method=method, headers=h)
        try:
            with urllib.request.urlopen(req) as r:
                return r.status, json.loads(r.read())
        except urllib.error.HTTPError as e:
            raw = e.read()
            return e.code, (json.loads(raw) if raw else {})

    def upload(self, data, name="report.pdf", mime="application/pdf", when_ms=None, token=TOKEN, sha=None):
        sha = sha if sha is not None else hashlib.sha256(data).hexdigest()
        headers = {"X-File-Sha256": sha, "X-File-Name": urllib.parse.quote(name), "X-File-Mime": mime,
                   "Content-Type": "application/octet-stream"}
        if when_ms is not None:
            headers["X-File-Ts"] = str(when_ms)
        return self.req("POST", "/ingest/file", data, headers, token)

    def test_needs_the_token(self):
        self.assertEqual(self.upload(b"x", token="wrong")[0], 401)

    def test_stores_the_bytes_under_their_hash_and_records_a_sighting(self):
        data = "분기 보고서 본문".encode("utf-8")
        sha = hashlib.sha256(data).hexdigest()
        when = int(__import__("datetime").datetime(2026, 10, 27, 9, 0).timestamp() * 1000)
        status, body = self.upload(data, name="보고서.pdf", when_ms=when)
        self.assertEqual(status, 200)
        self.assertEqual(body, {"sha256": sha, "stored": True, "duplicate": False})
        blob = self.root / "raw" / "files" / sha[:2] / sha
        self.assertTrue(blob.exists())
        self.assertEqual(blob.read_bytes(), data)
        line = (self.root / "raw" / "pcfile" / "2026-10-27.ndjson").read_text(encoding="utf-8").splitlines()[0]
        rec = json.loads(line)
        self.assertEqual((rec["type"], rec["sha256"], rec["name"], rec["size"]), ("file", sha, "보고서.pdf", len(data)))

    def test_resend_keeps_one_blob_but_records_each_sighting(self):
        data = b"same file seen twice"
        when1 = int(__import__("datetime").datetime(2026, 10, 28, 8, 0).timestamp() * 1000)
        when2 = int(__import__("datetime").datetime(2026, 10, 28, 15, 0).timestamp() * 1000)
        self.assertEqual(self.upload(data, name="a.txt", when_ms=when1)[1]["stored"], True)
        status, body = self.upload(data, name="a.txt", when_ms=when2)
        self.assertEqual(status, 200)
        self.assertEqual((body["stored"], body["duplicate"]), (False, True))
        lines = (self.root / "raw" / "pcfile" / "2026-10-28.ndjson").read_text(encoding="utf-8").splitlines()
        self.assertEqual(len(lines), 2)

    def test_hash_mismatch_is_refused_and_nothing_is_written(self):
        data = b"the body"
        status, body = self.upload(data, name="b.txt", sha="0" * 64)
        self.assertEqual(status, 400)
        self.assertNotIn("0" * 64, [p.name for p in (self.root / "raw" / "files" / "00").glob("*")] if (self.root / "raw" / "files" / "00").exists() else [])

    def test_head_tells_whether_a_blob_is_already_kept(self):
        data = b"ask before upload"
        sha = hashlib.sha256(data).hexdigest()
        self.assertEqual(self.req("GET", f"/ingest/file/{sha}")[1], {"sha256": sha, "present": False})
        self.upload(data, name="c.txt")
        self.assertEqual(self.req("GET", f"/ingest/file/{sha}")[1], {"sha256": sha, "present": True})

    def test_sighting_only_records_a_line_without_re_sending_bytes(self):
        data = b"a kept deliverable seen again"
        sha = hashlib.sha256(data).hexdigest()
        when1 = int(__import__("datetime").datetime(2026, 10, 30, 9, 0).timestamp() * 1000)
        when2 = int(__import__("datetime").datetime(2026, 10, 30, 17, 0).timestamp() * 1000)
        self.upload(data, name="d.txt", when_ms=when1)
        headers = {"X-File-Sha256": sha, "X-File-Name": urllib.parse.quote("d.txt"), "X-File-Mime": "text/plain",
                   "X-File-Sighting": "1", "X-File-Ts": str(when2)}
        status, body = self.req("POST", "/ingest/file", b"", headers)
        self.assertEqual(status, 200)
        self.assertEqual((body["stored"], body["duplicate"]), (False, True))
        lines = (self.root / "raw" / "pcfile" / "2026-10-30.ndjson").read_text(encoding="utf-8").splitlines()
        self.assertEqual(len(lines), 2)
        # the size on a sighting comes from the kept blob, not the caller
        self.assertEqual(json.loads(lines[1])["size"], len(data))

    def test_sighting_only_for_an_unknown_blob_is_refused(self):
        headers = {"X-File-Sha256": "1" * 64, "X-File-Name": "x", "X-File-Sighting": "1"}
        self.assertEqual(self.req("POST", "/ingest/file", b"", headers)[0], 409)

    def test_sighting_with_a_malformed_sha_is_refused_not_a_path(self):
        # an empty or path-like sha must never reach the blob path; it is refused as "not present"
        for bad in ("", "../../etc/passwd", "zz", "g" * 64):
            headers = {"X-File-Sha256": bad, "X-File-Name": "x", "X-File-Sighting": "1"}
            self.assertEqual(self.req("POST", "/ingest/file", b"", headers)[0], 409, bad)


if __name__ == "__main__":
    unittest.main()
