"""Settings of life-ingest: ~/.config/vault-sync/ingest.env (KEY=VALUE, mode 600) and where the raw data lives."""
import os
import sys
from pathlib import Path

ENV_FILE = Path.home() / ".config/vault-sync/ingest.env"


def load_env():
    cfg = {}
    if ENV_FILE.exists():
        for line in ENV_FILE.read_text(encoding="utf-8").splitlines():
            line = line.strip()
            if line and not line.startswith("#") and "=" in line:
                k, _, v = line.partition("=")
                cfg[k.strip()] = v.strip().strip('"').strip("'")
    return cfg


def data_root(cfg):
    if cfg.get("LIFE_DATA_DIR"):
        return Path(cfg["LIFE_DATA_DIR"]).expanduser()
    if sys.platform.startswith("win"):
        return Path(os.environ.get("LOCALAPPDATA", str(Path.home() / "AppData/Local"))) / "life"
    if sys.platform == "darwin":
        return Path.home() / "Library/Application Support/life"
    return Path(os.environ.get("XDG_DATA_HOME", str(Path.home() / ".local/share"))) / "life"
