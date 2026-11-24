"""Qwen3-Omni-30B-A3B-Captioner subprocess for the Lush worker (docs/WORKER_PROTOCOL.md, "Captioner subprocess").

stdin : one JSON line per track  {"id", "audio", "prompt"}
stdout: one JSON line per answer {"id", "text"} or {"id", "error"}; stdout carries ONLY protocol lines
stderr: all logs
Exits 0 when stdin closes. The model loads once at start-up.

The model takes audio only (no text prompt; the request's "prompt" is ignored) and answers in free text,
so captioner.json uses "mode": "text". The model card recommends clips of at most 30 s, so by default the
script sends a 30 s window from the middle of the track (QWEN3_CAPTION_SECONDS; 0 = whole track).
License: Apache 2.0. See README.md for what was self-tested on rtx6000.
"""
from __future__ import annotations

import json
import os
import sys
from typing import IO, Any, Optional

MODEL_DIR = os.environ.get("QWEN3_CAPTIONER_MODEL_DIR", "./weights/Qwen3-Omni-30B-A3B-Captioner")
MAX_NEW_TOKENS = int(os.environ.get("QWEN3_CAPTIONER_MAX_NEW_TOKENS", "1024"))
CLIP_SECONDS = float(os.environ.get("QWEN3_CAPTION_SECONDS", "30"))
SAMPLE_RATE = 16000
DEFAULT_PROMPT = None  # audio-only model


def log(msg: str) -> None:
    print(f"[qwen3-omni-captioner] {msg}", file=sys.stderr, flush=True)


def middle_window(n_samples: int, rate: int, seconds: float) -> tuple[int, int]:
    """[start, end) of a `seconds`-long window centred in the track; the whole track if shorter or seconds <= 0."""
    want = int(seconds * rate)
    if seconds <= 0 or n_samples <= want:
        return 0, n_samples
    start = (n_samples - want) // 2
    return start, start + want


# ---------- model part: heavy imports stay inside ----------
def load_model() -> Any:
    """Return an opaque handle (model, processor)."""
    if os.environ.get("QWEN3_CAPTIONER_FAKE_GENERATE") == "1":  # test hook
        print("junk from print at load", flush=True)
        return "FAKE"
    import torch  # noqa: F401  (fail early if missing)
    from transformers import Qwen3OmniMoeForConditionalGeneration, Qwen3OmniMoeProcessor

    processor = Qwen3OmniMoeProcessor.from_pretrained(MODEL_DIR)
    model = Qwen3OmniMoeForConditionalGeneration.from_pretrained(
        MODEL_DIR, torch_dtype="auto", device_map="cuda:0").eval()
    if hasattr(model, "disable_talker"):
        model.disable_talker()
    return model, processor


def generate(handle: Any, audio_path: str, prompt: Optional[str]) -> str:
    if handle == "FAKE":
        print("junk from print", flush=True)
        os.write(1, b"junk from os.write\n")
        return f"fake caption for {os.path.basename(audio_path)} é"
    import librosa
    import torch

    if not os.path.isfile(audio_path):
        raise FileNotFoundError(f"no such file: {audio_path}")
    model, processor = handle
    wav, _ = librosa.load(audio_path, sr=SAMPLE_RATE, mono=True)
    if wav.size == 0:
        raise ValueError("empty audio")
    a, b = middle_window(wav.size, SAMPLE_RATE, CLIP_SECONDS)
    wav = wav[a:b]
    conversation = [{"role": "user", "content": [{"type": "audio", "audio": audio_path}]}]
    text = processor.apply_chat_template(conversation, add_generation_prompt=True, tokenize=False)
    inputs = processor(text=text, audio=[wav], return_tensors="pt", padding=True,
                       use_audio_in_video=False).to(model.device)
    for k, v in inputs.items():
        if torch.is_tensor(v) and v.is_floating_point():
            inputs[k] = v.to(model.dtype)
    with torch.no_grad():
        out = model.generate(**inputs, max_new_tokens=MAX_NEW_TOKENS, return_audio=False,
                             thinker_return_dict_in_generate=True, use_audio_in_video=False)
    ids = out[0] if isinstance(out, tuple) else out
    ids = getattr(ids, "sequences", ids)
    return processor.batch_decode(ids[:, inputs["input_ids"].shape[1]:], skip_special_tokens=True)[0].strip()


# ---------- protocol loop (no torch needed) ----------
def _emit(out: IO[str], obj: dict) -> None:
    out.write(json.dumps(obj, ensure_ascii=False) + "\n")
    out.flush()


def _free_gpu_cache() -> None:
    try:
        import torch
        if torch.cuda.is_available():
            torch.cuda.empty_cache()
    except Exception:  # noqa: BLE001 - torch absent (tests) or CUDA unusable
        pass


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
        _free_gpu_cache()
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
    # Model libs may print to fd 1; keep the real stdout for protocol lines only.
    _real_out = os.fdopen(os.dup(1), "w", encoding="utf-8", buffering=1)
    os.dup2(2, 1)
    sys.stdout = sys.stderr
    sys.exit(main(stdout=_real_out))
