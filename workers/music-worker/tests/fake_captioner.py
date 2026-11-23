"""Fake captioner subprocess (JSON lines on stdin/stdout). Behaviour follows the audio file name:
good -> valid JSON, bad -> "not json", empty -> "", crash -> exit 1 on the first request only.

Env: FAKE_CAPTIONER_STATE (dir for the crash marker), FAKE_CAPTIONER_LOG (file; one audio name per request).
"""
import json
import os
import sys
from pathlib import Path

GOOD = {"mood": "calm", "instruments": "piano", "vocals": "Instrumental", "caption": "A calm piano piece."}

for line in sys.stdin:
    req = json.loads(line)
    name = Path(req["audio"]).name
    log = os.environ.get("FAKE_CAPTIONER_LOG")
    if log:
        with open(log, "a", encoding="utf-8") as f:
            f.write(name + "\n")
    print(f"processing {name}", file=sys.stderr, flush=True)
    if "crash" in name:
        marker = Path(os.environ.get("FAKE_CAPTIONER_STATE", ".")) / "crashed"
        if not marker.exists():
            marker.write_text("1")
            sys.exit(1)
    if "good" in name:
        text = json.dumps(GOOD)
    elif "bad" in name:
        text = "not json"
    elif "empty" in name:
        text = ""
    else:
        text = json.dumps(GOOD)
    print("stray non-json line", flush=True)
    print(json.dumps({"id": req["id"], "text": text}), flush=True)
