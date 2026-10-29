"""Turns a stream of "what is in front, and how long since the last input" samples into closed time segments.

Pure logic with no OS calls, so it can be tested anywhere. A segment is closed (and returned) when the window changes,
when the user goes away or comes back, or when it has lasted max_segment_s.
"""
import re
import socket

from life_agent.config import HIDDEN_TITLE

HOST = socket.gethostname()


class Segmenter:
    def __init__(self, cfg, host=HOST):
        self.cfg, self.host = cfg, host
        self.cur = None   # {"kind", "app", "title", "start"}
        self._hide_apps = {a.lower() for a in cfg["hide_title_apps"]}
        self._hide_re = [re.compile(p, re.I) for p in cfg["hide_title_patterns"]]

    def _title(self, app, title):
        if app.lower() in self._hide_apps or any(r.search(title) for r in self._hide_re):
            return HIDDEN_TITLE
        return title

    def _close(self, end):
        c, self.cur = self.cur, None
        if not c or end <= c["start"]:
            return None
        dur = end - c["start"]
        if c["kind"] == "window" and dur < self.cfg["min_segment_s"]:
            return None
        rec = {"type": c["kind"], "ts": int(c["start"] * 1000), "end": int(end * 1000), "dur_s": int(dur), "host": self.host}
        if c["kind"] == "window":
            rec.update(app=c["app"], title=c["title"])
        else:
            rec["reason"] = c["reason"]
        return rec

    def feed(self, now, app, title, idle_s):
        """One sample taken at `now` (epoch seconds). app is "" when nothing is in front (locked screen, no window).
        Returns the segments closed by this sample (0, 1 or 2 records)."""
        away = app == "" or idle_s >= self.cfg["idle_after_s"]
        # the last real input, not the moment we noticed: the away period starts there
        since = max(now - idle_s, self.cur["start"] if self.cur else now)
        if away:
            reason = "locked" if app == "" else "idle"
            if self.cur and self.cur["kind"] == "idle" and self.cur["reason"] == reason:
                return self._cut(now)
            out = [self._close(since if app != "" else now)]
            self.cur = {"kind": "idle", "reason": reason, "start": since if app != "" else now}
            return [r for r in out if r]
        want = {"kind": "window", "app": app, "title": self._title(app, title)}
        if self.cur and all(self.cur.get(k) == v for k, v in want.items()):
            return self._cut(now)
        # coming back from away: the new window starts at the first input after the break
        began = now if not self.cur or self.cur["kind"] == "window" else max(now - idle_s, self.cur["start"])
        out = [self._close(began)]
        self.cur = {**want, "start": began}
        return [r for r in out if r]

    def _cut(self, now):
        if self.cur and now - self.cur["start"] >= self.cfg["max_segment_s"]:
            kind = dict(self.cur)
            rec = self._close(now)
            self.cur = {**kind, "start": now}
            return [rec] if rec else []
        return []

    def finish(self, now):
        """Close the running segment (agent is stopping)."""
        rec = self._close(now)
        return [rec] if rec else []
