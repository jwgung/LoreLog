import unittest

import _env  # noqa: F401
from life_agent import config
from life_agent.segments import Segmenter

CFG = {**config.DEFAULTS, "idle_after_s": 300, "max_segment_s": 600, "min_segment_s": 3}


def run(samples, cfg=CFG):
    """samples: (t, app, title, idle_s). Returns every record closed, including the last one (finish at the final t)."""
    s, out = Segmenter(cfg, host="H"), []
    for t, app, title, idle in samples:
        out += s.feed(t, app, title, idle)
    return out + s.finish(samples[-1][0])


class Segments(unittest.TestCase):
    def test_window_change_closes_the_segment(self):
        out = run([(0, "a.exe", "x", 0), (10, "a.exe", "x", 0), (20, "b.exe", "y", 0), (40, "b.exe", "y", 0)])
        self.assertEqual([(r["app"], r["dur_s"]) for r in out], [("a.exe", 20), ("b.exe", 20)])
        self.assertEqual(out[0]["ts"], 0)
        self.assertEqual(out[0]["end"], 20000)
        self.assertEqual(out[0]["host"], "H")

    def test_title_change_in_the_same_app_is_a_new_segment(self):
        out = run([(0, "chrome.exe", "메일", 0), (30, "chrome.exe", "견적서", 0), (60, "chrome.exe", "견적서", 0)])
        self.assertEqual([r["title"] for r in out], ["메일", "견적서"])

    def test_alt_tab_flicker_is_not_recorded(self):
        out = run([(0, "a.exe", "x", 0), (30, "b.exe", "y", 0), (31, "a.exe", "x", 0), (60, "a.exe", "x", 0)])
        self.assertEqual([(r["app"], r["dur_s"]) for r in out], [("a.exe", 30), ("a.exe", 29)])

    def test_long_stay_is_cut_into_pieces(self):
        out = run([(t, "a.exe", "x", 0) for t in range(0, 1500, 100)])
        self.assertEqual([r["dur_s"] for r in out], [600, 600, 200])

    def test_away_starts_at_the_last_input_not_when_we_noticed(self):
        # last input at t=100; noticed at t=400 (idle 300). The window ends at 100 and the idle period starts there.
        out = run([(0, "a.exe", "x", 0), (100, "a.exe", "x", 0), (400, "a.exe", "x", 300), (700, "a.exe", "x", 600)])
        win, idle = out[0], out[1]
        self.assertEqual((win["type"], win["dur_s"]), ("window", 100))
        self.assertEqual((idle["type"], idle["ts"], idle["end"], idle["reason"]), ("idle", 100000, 700000, "idle"))

    def test_coming_back_starts_the_new_window_at_the_first_input(self):
        # working until t=200 (last input), away, back at t=1000 (idle 0), still there at 1030
        out = run([(0, "a.exe", "x", 0), (200, "a.exe", "x", 0), (500, "a.exe", "x", 300), (1000, "a.exe", "x", 0), (1030, "a.exe", "x", 0)])
        self.assertEqual([(r["type"], r["ts"] // 1000, r["end"] // 1000) for r in out],
                         [("window", 0, 200), ("idle", 200, 1000), ("window", 1000, 1030)])

    def test_away_from_the_start_has_no_zero_length_window(self):
        out = run([(0, "a.exe", "x", 400), (100, "a.exe", "x", 500), (200, "a.exe", "x", 0), (230, "a.exe", "x", 0)])
        self.assertEqual([(r["type"], r["dur_s"]) for r in out], [("idle", 200), ("window", 30)])

    def test_locked_screen_is_recorded_as_locked(self):
        out = run([(0, "a.exe", "x", 0), (50, "", "", 50), (200, "", "", 200)])
        self.assertEqual([(r["type"], r.get("reason")) for r in out], [("window", None), ("idle", "locked")])
        self.assertEqual(out[1]["dur_s"], 150)

    def test_private_titles_are_hidden(self):
        out = run([(0, "msedge.exe", "InPrivate - 은행", 0), (30, "KeePass.exe", "비밀번호 db", 0), (60, "a.exe", "x", 0), (90, "a.exe", "x", 0)])
        self.assertEqual([(r["app"], r["title"]) for r in out[:2]], [("msedge.exe", config.HIDDEN_TITLE), ("KeePass.exe", config.HIDDEN_TITLE)])


if __name__ == "__main__":
    unittest.main()
