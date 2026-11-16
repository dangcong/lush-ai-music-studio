"""Read-only check of this machine: Python, GPU, torch, audio decoding, data root, ACE-Step checkout.

Usage: python scripts/doctor.py [--json]
Run with --help for every option. Details: docs/TRAINING_GUIDE.md.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["doctor", *sys.argv[1:]]))
