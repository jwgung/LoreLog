"""Settings file of the agent. Secrets live here (mode 600 where the OS has modes), never in the repo."""
import json
import os
import sys
from pathlib import Path

DEFAULTS = {
    "url": "",                  # e.g. https://<machine>.<tailnet>.ts.net:48765  (no trailing slash needed)
    "token": "",                # the server's INGEST_TOKEN; put it in the file yourself
    "source": "pc",             # records go to <url>/ingest/<source>
    "interval_s": 5,            # how often the foreground window is looked at
    "idle_after_s": 300,        # no keyboard/mouse for this long counts as away
    "max_segment_s": 600,       # a long stay is cut into pieces of this length so nothing is lost on a crash
    "min_segment_s": 3,         # shorter window visits (alt-tab flicker) are not recorded
    "flush_every_s": 60,
    # Windows of these apps are recorded by app name only; their titles are replaced.
    "hide_title_apps": ["KeePass.exe", "KeePassXC.exe", "1Password.exe", "Bitwarden.exe"],
    # A title matching one of these regexes is replaced too (private browsing windows).
    "hide_title_patterns": ["InPrivate", "시크릿", "Incognito", "Private Browsing"],
}
HIDDEN_TITLE = "(제목 숨김)"


def home():
    if os.environ.get("LIFE_AGENT_HOME"):
        return Path(os.environ["LIFE_AGENT_HOME"])
    if sys.platform.startswith("win"):
        return Path(os.environ.get("APPDATA", str(Path.home() / "AppData/Roaming"))) / "life-agent"
    if sys.platform == "darwin":
        return Path.home() / "Library/Application Support/life-agent"
    return Path(os.environ.get("XDG_CONFIG_HOME", str(Path.home() / ".config"))) / "life-agent"


def config_path():
    return home() / "config.json"


def load():
    cfg = dict(DEFAULTS)
    try:
        cfg.update(json.loads(config_path().read_text(encoding="utf-8")))
    except OSError:
        pass
    return cfg


def write_template():
    """Create config.json with the defaults if it does not exist yet; return its path."""
    p = config_path()
    if not p.exists():
        p.parent.mkdir(parents=True, exist_ok=True)
        p.write_text(json.dumps(DEFAULTS, ensure_ascii=False, indent=2), encoding="utf-8")
        try:
            p.chmod(0o600)
        except OSError:
            pass  # Windows: the profile folder is already private to the user
    return p
