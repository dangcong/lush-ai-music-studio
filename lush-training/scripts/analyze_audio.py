"""Measure every segment of a dataset version (cached by segment SHA-256). Evidence for captions, not judgements.

Usage: python scripts/analyze_audio.py --dataset <name> --version <v>
Run with --help for every option. Details: docs/TRAINING_GUIDE.md.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["analyze", *sys.argv[1:]]))
