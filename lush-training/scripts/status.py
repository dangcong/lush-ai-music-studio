"""Show runs, progress, checkpoints and the trainer log. Reads files only.

Usage: python scripts/status.py [--run <run_id>] [--log 50]
Run with --help for every option. Details: docs/TRAINING_GUIDE.md.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["status", *sys.argv[1:]]))
