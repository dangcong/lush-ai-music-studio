"""Fake scorer subprocess (JSON lines on stdin/stdout). Behaviour follows the audio file name:
`err` -> {"error": ...}; `quality` -> {"quality": 0.5}; anything else -> a fixed embedding.
Stray non-JSON lines go to stdout on purpose (the client must ignore them)."""
import json
import sys
from pathlib import Path

for line in sys.stdin:
    req = json.loads(line)
    name = Path(req["path"]).name
    print("stray non-json line", flush=True)
    if "err" in name:
        print(json.dumps({"error": f"cannot read {name}"}), flush=True)
    elif "quality" in name:
        print(json.dumps({"quality": 0.5}), flush=True)
    else:
        print(json.dumps({"embedding": [1.0, 2.0, 3.0]}), flush=True)
