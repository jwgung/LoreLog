"""One backend per OS. A backend has sample() -> (app name, window title, seconds since the last input)."""
import sys


def load():
    if sys.platform.startswith("win"):
        from life_agent.backends import windows
        return windows
    raise NotImplementedError(f"{sys.platform} is not supported yet (Windows only for now)")
