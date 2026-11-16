"""Scan a music folder, record vocal reviews, lock an immutable dataset version. Never modifies source audio.

Usage: python scripts/prepare_dataset.py scan|review|build --config configs/dataset.yaml [--version v1]
Run with --help for every option. Details: docs/TRAINING_GUIDE.md.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["dataset", *sys.argv[1:]]))
