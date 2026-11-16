"""Build or verify an inference package for Lush Studio. Copies nothing off this machine.

Usage: python scripts/export_model.py export --run <run_id> --checkpoint <step> --name <name> | verify --package <dir>
Run with --help for every option. Details: docs/TRAINING_GUIDE.md.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["export", *sys.argv[1:]]))
