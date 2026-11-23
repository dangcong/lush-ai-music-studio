"""Draft captions for a dataset version with a captioner (or a folder label). Never approves.

Usage: python scripts/draft_captions.py --dataset lofi --version v1 --captioner folder-label --template "lo-fi instrumental"
       python scripts/draft_captions.py --dataset lofi --version v1 --captioner acestep-captioner --model-dir DIR --revision HASH
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["draft", *sys.argv[1:]]))
