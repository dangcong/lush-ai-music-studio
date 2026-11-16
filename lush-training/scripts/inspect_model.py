"""Read a pretrained checkpoint (header only, no GPU): components, parameter counts, compatible backend.

Usage: python scripts/inspect_model.py <checkpoint> [--hash] [--method full_decoder]
Run with --help for every option. Details: docs/TRAINING_GUIDE.md.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["inspect", *sys.argv[1:]]))
