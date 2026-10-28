"""Where the assistant reads and writes. Every path can be overridden by an environment variable."""
import os
from pathlib import Path

VAULT = Path(os.environ.get("LIFE_VAULT", Path.home() / "Obsidian"))
DATA = Path(os.environ.get("LIFE_DATA_DIR", Path.home() / ".local/share/life"))
LLM_URL = os.environ.get("LIFE_LLM_URL", "http://10.100.3.232:8000/v1")
LLM_MODEL = os.environ.get("LIFE_LLM_MODEL", "ultivis-vl")

AI_BEGIN, AI_END = "<!-- ai:begin -->", "<!-- ai:end -->"
# Phone system apps whose notifications and app events say nothing about the user's day.
NOISE_PKGS = ("com.android.systemui", "com.sec.android.app.launcher", "com.samsung.android.honeyboard")

ALLOW_FILE = Path(os.environ.get("LIFE_DIARY_CONFIG", Path.home() / ".config/vault-sync/diary.json"))
TODO_FILE = DATA / "todos.json"
