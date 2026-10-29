"""The collection loop: sample, segment, spool, send. The clock and the OS backend are passed in so it can be tested."""
import time

from life_agent import config, sender, spool
from life_agent.segments import Segmenter

LOG_MAX = 1_000_000


def log(msg):
    """Append to agent.log (pythonw has no console). Never write the token here."""
    try:
        p = config.home() / "agent.log"
        p.parent.mkdir(parents=True, exist_ok=True)
        if p.exists() and p.stat().st_size > LOG_MAX:
            p.write_text("", encoding="utf-8")
        with open(p, "a", encoding="utf-8") as f:
            f.write(f"{time.strftime('%Y-%m-%d %H:%M:%S')} {msg}\n")
    except OSError:
        pass


class Agent:
    def __init__(self, cfg, backend, clock=time.time):
        self.cfg, self.backend, self.clock = cfg, backend, clock
        self.seg = Segmenter(cfg)
        self.last = None          # time of the previous sample
        self.last_flush = clock()
        self.failing = ""         # last send error, so the log is not flooded

    def step(self):
        now = self.clock()
        # the machine slept or the loop stalled: what happened in the gap is unknown, so end the segment where it was last seen
        if self.last is not None and now - self.last > max(30, 4 * self.cfg["interval_s"]):
            spool.add(self.seg.finish(self.last))
        self.last = now
        try:
            app, title, idle = self.backend.sample()
        except Exception as e:  # noqa: BLE001 - one bad sample must not stop the agent
            log(f"sample failed: {type(e).__name__}: {e}")
            return
        spool.add(self.seg.feed(now, app, title, idle))
        if now - self.last_flush >= self.cfg["flush_every_s"]:
            self.flush()

    def flush(self):
        self.last_flush = self.clock()
        sent, err = sender.send_spool(self.cfg)
        if sent:
            log(f"sent {sent} records")
        if err != self.failing:
            log(f"send problem: {err}" if err else "send recovered")
            self.failing = err
        return sent, err

    def stop(self):
        spool.add(self.seg.finish(self.clock()))
        return self.flush()
