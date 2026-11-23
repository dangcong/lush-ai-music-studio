"""Export a locked dataset version + caption version for Lush Studio's Train Style worker.

Usage: python scripts/export_studio_dataset.py --dataset lofi --version v1 --caption-version c1 \
         --studio-root /media/ai-rtx6000/AI_DATA/AiStudio/data/datasets --key lofi-pilot/v1
Never overwrites an existing export. Details: docs/TRAINING_GUIDE.md.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["studio-export", *sys.argv[1:]]))
