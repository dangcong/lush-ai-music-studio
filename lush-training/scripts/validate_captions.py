"""Create caption drafts, edit, validate against the dataset manifest, lock a caption version.

Usage: python scripts/validate_captions.py init|validate|set|lock --dataset <name> --version <v>
Run with --help for every option. Details: docs/TRAINING_GUIDE.md.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["captions", *sys.argv[1:]]))
