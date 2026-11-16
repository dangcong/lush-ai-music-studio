"""Create a run, preflight, preprocess tensors, start / resume / stop the trainer. GPU actions need --confirm-gpu.

Usage: python scripts/train.py create|preflight|preprocess|start|resume|stop ...
Run with --help for every option. Details: docs/TRAINING_GUIDE.md.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["train", *sys.argv[1:]]))
