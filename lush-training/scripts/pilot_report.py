"""Write `<version>/review/pilot-report.md` from the review log and print it.

Usage: python scripts/pilot_report.py --dataset lofi --version v1
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["pilot-report", *sys.argv[1:]]))
