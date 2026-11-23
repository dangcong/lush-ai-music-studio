"""Caption review page (localhost only) and caption carry-over between dataset versions.

Usage: python scripts/review_server.py serve --dataset lofi --version v1 [--port 8765]
       then on the dev PC: ssh -L 8765:127.0.0.1:8765 rtx6000  and open http://127.0.0.1:8765/
       python scripts/review_server.py carry-over --dataset lofi --version v2 --from-version v1
"""
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "src"))

from lush_training.cli import main  # noqa: E402

if __name__ == "__main__":
    sys.exit(main(["review", *sys.argv[1:]]))
