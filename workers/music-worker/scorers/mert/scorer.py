"""MERT embedding scorer subprocess for the Lush worker (docs/WORKER_PROTOCOL.md, "Scorer subprocess").

stdin : one JSON line per file  {"path": "<audio file>"}
stdout: exactly one JSON line per input line: {"embedding": [float, ...]} or {"error": "..."}; stdout carries ONLY protocol lines
stderr: all logs
Exits 0 when stdin closes. The model loads once at start-up.

LUSH_SCORER_STUB=1 uses a deterministic fake score and never imports torch / transformers / audiobox.
Only the protocol loop is verified by tests; `load_model` / `score_file` follow the library docs but have
NOT been run (chua kiem chung) - see README.md.
"""
from __future__ import annotations

import hashlib
import json
import os
import sys
from typing import IO, Any

MODEL_ID = os.environ.get("MERT_MODEL_ID", "m-a-p/MERT-v1-330M")
# Pinned: the revision installed and self-tested on rtx6000 (remote code runs with trust_remote_code).
REVISION = os.environ.get("MERT_REVISION", "5240c2708a5acaee1007f43fb9735c7dcd0b78c9")
SECONDS = 60.0  # only the first minute of a file
STUB = os.environ.get("LUSH_SCORER_STUB") == "1"


def log(msg: str) -> None:
    print(f"[mert] {msg}", file=sys.stderr, flush=True)


# ---------- model part: self-tested on rtx6000 2026-11-24; heavy imports stay inside ----------
def load_model() -> Any:
    if STUB:
        if os.environ.get("LUSH_SCORER_STUB_NOISY") == "1":  # test hook: libraries that print to stdout
            print("junk from print at load", flush=True)
        return "STUB"
    import torch
    from transformers import AutoModel, Wav2Vec2FeatureExtractor

    device = "cuda" if torch.cuda.is_available() else "cpu"
    model = AutoModel.from_pretrained(MODEL_ID, revision=REVISION, trust_remote_code=True).to(device).eval()
    extractor = Wav2Vec2FeatureExtractor.from_pretrained(MODEL_ID, revision=REVISION, trust_remote_code=True)
    return model, extractor, device


def score_file(handle: Any, path: str) -> list[float]:
    if handle == "STUB":
        if os.environ.get("LUSH_SCORER_STUB_NOISY") == "1":
            print("junk from print", flush=True)
            os.write(1, b"junk from os.write\n")
        with open(path, "rb") as f:  # raises if the file is missing, like the real code would
            digest = hashlib.sha256(f.read()).digest()
        return [(digest[i] / 255.0) * 2.0 - 1.0 for i in range(8)]
    import librosa
    import torch

    model, extractor, device = handle
    rate = int(extractor.sampling_rate)  # 24000 for MERT-v1-330M
    wav, _ = librosa.load(path, sr=rate, mono=True, duration=SECONDS)  # resample + first 60 s
    if wav.size == 0:
        raise ValueError("empty audio")
    inputs = extractor(wav, sampling_rate=rate, return_tensors="pt").to(device)
    with torch.no_grad():
        out = model(**inputs)
    # Last layer. MERT's remote code ignores output_hidden_states under transformers 5, so use last_hidden_state.
    last = out.last_hidden_state.squeeze(0)  # (time, 1024)
    return [float(x) for x in last.mean(dim=0).float().cpu().tolist()]


# ---------- protocol loop (no torch needed) ----------
def _emit(out: IO[str], obj: dict) -> None:
    out.write(json.dumps(obj, ensure_ascii=False) + "\n")
    out.flush()


def _handle(line: str, handle: Any, out: IO[str]) -> None:
    try:
        req = json.loads(line)
        if not isinstance(req, dict):
            raise ValueError("request is not a JSON object")
        path = req.get("path")
        if not isinstance(path, str) or not path:
            raise ValueError("missing 'path'")
        if not os.path.isfile(path):
            raise FileNotFoundError(f"no such file: {path}")
        value = score_file(handle, path)
        _emit(out, {"embedding": value})
    except Exception as e:  # noqa: BLE001 - one bad file must not kill the process
        log(f"error: {type(e).__name__}: {e}")
        _emit(out, {"error": f"{type(e).__name__}: {e}"})


def main(stdin: IO[str] = None, stdout: IO[str] = None) -> int:
    stdin = stdin or sys.stdin
    stdout = stdout or sys.stdout
    try:
        handle = load_model()
    except Exception as e:  # noqa: BLE001
        log(f"cannot load the model: {type(e).__name__}: {e}")
        return 1
    log("model ready")
    for line in stdin:
        line = line.strip()
        if line:
            _handle(line, handle, stdout)
    return 0


if __name__ == "__main__":
    # Model libs may print to fd 1; keep the real stdout for protocol lines only.
    _real_out = os.fdopen(os.dup(1), "w", encoding="utf-8", buffering=1)
    os.dup2(2, 1)
    sys.stdout = sys.stderr
    sys.stdin = open(sys.stdin.fileno(), "r", encoding="utf-8", errors="replace", closefd=False)
    sys.exit(main(stdout=_real_out))
