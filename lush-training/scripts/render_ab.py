"""Render base + each checkpoint against a running acestep-api for blind A/B listening.

Usage (in an approved window, worker stopped): ACESTEP_API_KEY=... python scripts/render_ab.py \
  --prompts configs/eval_prompts.lofi.yaml --eval-root /media/.../data/eval/lofi-pilot-v1/<run> \
  --checkpoint <run>/checkpoints/epoch_1 --checkpoint <run>/checkpoints/epoch_2 --run-id <run> --confirm-gpu
Afterwards restart acestep-api (drops test adapters), then `evaluate`-style ab/report per folder.
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["render-ab", *sys.argv[1:]]))
