"""Autopilot: run one job file end to end (dataset -> captions -> training -> selected checkpoint -> package).

Usage: python scripts/autopilot.py run|status --job configs/autopilot.yaml [--detach]
The job file carries the user's standing authorization for that job. Details: docs/TRAINING_GUIDE.md, section 10.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["auto", *sys.argv[1:]]))
