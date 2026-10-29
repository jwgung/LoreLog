import json
import sys
import threading
import unittest
from pathlib import Path

import _env  # noqa: F401
sys.path.insert(0, str(Path(__file__).resolve().parent.parent.parent / "server"))
from life.ingest.http import serve
from life_agent import config, spool
from life_agent.agent import Agent

TOKEN = "agent-test-token-0123456789"


class Clock:
    def __init__(self, t=1_800_000_000):
        self.t = t

    def __call__(self):
        return self.t


class Backend:
    def __init__(self):
        self.now = ("a.exe", "문서", 0)

    def sample(self):
        return self.now


class AgentToServer(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.root = _env.HOME / "server-data"
        cls.srv = serve("127.0.0.1", 0, cls.root, TOKEN, "personal", 1024 * 1024)
        cls.url = f"http://127.0.0.1:{cls.srv.server_address[1]}"
        threading.Thread(target=cls.srv.serve_forever, daemon=True).start()

    @classmethod
    def tearDownClass(cls):
        cls.srv.shutdown()

    def setUp(self):
        spool.path().unlink(missing_ok=True)
        self.cfg = {**config.DEFAULTS, "url": self.url, "token": TOKEN, "flush_every_s": 60}
        self.clock, self.backend = Clock(), Backend()
        self.agent = Agent(self.cfg, self.backend, self.clock)

    def tick(self, seconds, n=1):
        for _ in range(n):
            self.clock.t += seconds
            self.agent.step()

    def stored(self):
        out = []
        for f in sorted((self.root / "raw" / "pc").glob("*.ndjson")):
            out += [json.loads(line) for line in f.read_text(encoding="utf-8").splitlines()]
        return out

    def test_records_reach_the_server_with_the_agent_fields(self):
        self.tick(5, 6)
        self.backend.now = ("b.exe", "표", 0)
        self.tick(5, 4)
        self.agent.stop()
        recs = [r for r in self.stored() if r.get("host") == self.agent.seg.host and r["ts"] >= 1_800_000_000_000]
        self.assertEqual([(r["app"], r["title"]) for r in recs][-2:], [("a.exe", "문서"), ("b.exe", "표")])
        self.assertEqual(recs[-1]["_src"], "pc")
        self.assertEqual(spool.read(), [], "delivered lines leave the spool")

    def test_nothing_is_lost_while_the_server_refuses_and_a_resend_adds_nothing(self):
        self.cfg["token"] = "wrong-token-0123456789"
        self.tick(5, 4)
        self.agent.stop()
        pending = spool.read()
        self.assertTrue(pending, "undelivered records stay in the spool")
        self.cfg["token"] = TOKEN
        sent, err = self.agent.flush()
        self.assertEqual((sent, err), (len(pending), ""))
        self.assertEqual(self.agent.flush(), (0, ""))

    def test_sleep_gap_ends_the_segment_where_it_was_last_seen(self):
        self.cfg["url"] = ""                 # keep everything in the spool so this test reads only its own records
        self.tick(5, 4)                      # first seen at t0+5, last seen at t0+20
        self.tick(3600)                      # the laptop slept for an hour
        self.agent.stop()
        windows = [json.loads(line) for line in spool.read() if '"window"' in line]
        self.assertEqual(windows[0]["dur_s"], 15, "the hour of sleep is not counted as work")
        self.assertEqual(windows[0]["end"], (1_800_000_000 + 20) * 1000)

    def test_missing_url_is_reported_not_raised(self):
        self.cfg["url"] = ""
        self.tick(5, 3)
        self.assertIn("not set", self.agent.flush()[1])


if __name__ == "__main__":
    unittest.main()
