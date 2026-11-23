"""MOSS-Music captioner subprocess for the Lush worker (docs/WORKER_PROTOCOL.md, "Captioner subprocess").

stdin : one JSON line per track  {"id", "audio", "prompt"}   (prompt may be null -> DEFAULT_PROMPT)
stdout: one JSON line per answer {"id", "text"} or {"id", "error"}; stdout carries ONLY protocol lines
stderr: all logs
Exits 0 when stdin closes. The model loads once at start-up.

Only the protocol loop (`main`) is verified by tests. `load_model` / `generate` follow the author's
infer.py but have NOT been run (no GPU run yet) - see README.md, "chua kiem chung".
"""
from __future__ import annotations

import json
import os
import sys
from typing import IO, Any, Optional

MODEL_DIR = os.environ.get("MOSS_MUSIC_MODEL_DIR", "./weights/MOSS-Music-8B-Instruct")
# Clone of https://github.com/OpenMOSS/MOSS-Music (infer.py imports `src.*` from it).
REPO_DIR = os.environ.get("MOSS_MUSIC_REPO_DIR", "")
MAX_NEW_TOKENS = int(os.environ.get("MOSS_MUSIC_MAX_NEW_TOKENS", "1024"))
SAMPLING = {"temperature": 1.0, "top_p": 0.8, "top_k": 50}  # values from the author's infer.py
DEFAULT_PROMPT = (
    'Listen to this track and answer with JSON only: {"mood": "...", "instruments": "...", '
    '"vocals": "Instrumental or describe any voice", "caption": "one English paragraph under 80 words '
    'describing genre, instruments, mood and texture; do not state BPM or key"}')


def log(msg: str) -> None:
    print(f"[moss-music] {msg}", file=sys.stderr, flush=True)


# ---------- model part: UNVERIFIED (chua kiem chung), heavy imports stay inside ----------
def load_model() -> Any:
    """Return an opaque handle (model, processor, load_audio). Mirrors infer.py of OpenMOSS/MOSS-Music."""
    if REPO_DIR and REPO_DIR not in sys.path:
        sys.path.insert(0, REPO_DIR)
    import torch  # noqa: F401  (fail early if missing)
    from src.audio_io import load_audio
    from src.modeling_moss_music import MossMusicModel
    from src.processing_moss_music import MossMusicProcessor

    model = MossMusicModel.from_pretrained(
        MODEL_DIR, trust_remote_code=True, torch_dtype="auto", device_map="cuda:0").eval()
    processor = MossMusicProcessor.from_pretrained(
        MODEL_DIR, trust_remote_code=True, enable_time_marker=True)
    return model, processor, load_audio


def generate(handle: Any, audio_path: str, prompt: str) -> str:
    import torch

    model, processor, load_audio = handle
    audio = load_audio(audio_path, sample_rate=processor.config.mel_sr)
    batch = processor(text=prompt, audios=[audio], return_tensors="pt").to(model.device)
    if batch.get("audio_data") is not None:
        batch["audio_data"] = batch["audio_data"].to(model.dtype)
    batch["audio_input_mask"] = batch["input_ids"] == processor.audio_token_id
    with torch.no_grad():
        out = model.generate(**batch, max_new_tokens=MAX_NEW_TOKENS, do_sample=True, num_beams=1,
                             use_cache=True, **SAMPLING)
    return processor.decode(out[0, batch["input_ids"].shape[1]:], skip_special_tokens=True)


# ---------- protocol loop (no torch needed) ----------
def _emit(out: IO[str], obj: dict) -> None:
    out.write(json.dumps(obj, ensure_ascii=False) + "\n")
    out.flush()


def _handle(line: str, handle: Any, out: IO[str]) -> None:
    req_id: Optional[Any] = None
    try:
        req = json.loads(line)
        if not isinstance(req, dict):
            raise ValueError("request is not a JSON object")
        req_id = req.get("id")
        audio = req.get("audio")
        if not isinstance(audio, str) or not audio:
            raise ValueError("missing 'audio'")
        prompt = req.get("prompt") or DEFAULT_PROMPT
        text = generate(handle, audio, prompt)
        _emit(out, {"id": req_id, "text": text})
    except Exception as e:  # noqa: BLE001 - one bad track must not kill the process
        log(f"error on id={req_id!r}: {e}")
        _emit(out, {"id": req_id, "error": f"{type(e).__name__}: {e}"})


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
    sys.exit(main())
