"""Plan A/B renders, check rendered audio, build a blind listening sheet, write the evaluation report.

Usage: python scripts/evaluate.py plan|check|ab|report --run <run_id> --checkpoint step-00001000
Run with --help for every option. Details: docs/TRAINING_GUIDE.md.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["evaluate", *sys.argv[1:]]))
