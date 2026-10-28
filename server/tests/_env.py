"""Point the assistant at a throwaway vault and data dir before any life module is imported."""
import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(tempfile.mkdtemp(prefix="life-test-"))
os.environ["LIFE_VAULT"] = str(ROOT / "vault")
os.environ["LIFE_DATA_DIR"] = str(ROOT / "data")
os.environ["LIFE_DIARY_CONFIG"] = str(ROOT / "diary.json")
(ROOT / "vault" / "Mail" / "2026").mkdir(parents=True)
(ROOT / "vault" / "Events").mkdir(parents=True)
(ROOT / "data" / "raw" / "lorelog").mkdir(parents=True)
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
