"""Run against a throwaway agent home."""
import os
import sys
import tempfile
from pathlib import Path

HOME = Path(tempfile.mkdtemp(prefix="life-agent-test-"))
os.environ["LIFE_AGENT_HOME"] = str(HOME)
sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
