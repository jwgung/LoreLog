"""Windows launcher without a console window (used by the sign-in autostart). Same as: python -m life_agent."""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from life_agent.cli import main
sys.exit(main())
