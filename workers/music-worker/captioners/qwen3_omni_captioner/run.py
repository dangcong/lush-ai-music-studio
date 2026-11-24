"""Qwen3-Omni-30B-A3B-Captioner subprocess for the Lush worker (docs/WORKER_PROTOCOL.md, "Captioner subprocess").

stdin : one JSON line per track  {"id", "audio", "prompt"}
stdout: one JSON line per answer {"id", "text"} or {"id", "error"}; stdout carries ONLY protocol lines
stderr: all logs
Exits 0 when stdin closes. The model loads once at start-up.

The model takes audio only (no text prompt; the request's "prompt" is ignored) and answers in free text,
so captioner.json uses "mode": "text". The model card recommends clips of at most 30 s, so the script trims
leading/trailing silence, captions three sections (Intro = first 30 s, Middle = central 30 s, Outro = last
30 s; equal thirds for tracks under 90 s; one pass for tracks under 30 s) and joins them as
"Intro: …\nMiddle: …\nOutro: …", each part cut at a sentence end to fit the 4000-character caption.
License: Apache 2.0. See README.md for what was self-tested on rtx6000.
"""
from __future__ import annotations

import json
import os
import sys
from typing import IO, Any, Optional

MODEL_DIR = os.environ.get("QWEN3_CAPTIONER_MODEL_DIR", "./weights/Qwen3-Omni-30B-A3B-Captioner")
MAX_NEW_TOKENS = int(os.environ.get("QWEN3_CAPTIONER_MAX_NEW_TOKENS", "1024"))
SECTION_SECONDS = float(os.environ.get("QWEN3_SECTION_SECONDS", "30"))
SECTION_CHARS = int(os.environ.get("QWEN3_SECTION_CHARS", "1200"))  # 3 x 1200 + labels < 4000 (caption limit)
SILENCE_DB = 40  # librosa.effects.trim top_db
SAMPLE_RATE = 16000
DEFAULT_PROMPT = None  # audio-only model


def log(msg: str) -> None:
    print(f"[qwen3-omni-captioner] {msg}", file=sys.stderr, flush=True)


def section_windows(n_samples: int, rate: int, seconds: float) -> list[tuple[str, int, int]]:
    """Intro / Middle / Outro windows [start, end) of at most `seconds` each; equal thirds when the track is
    shorter than three windows; one "Whole" window when it is not longer than one."""
    want = int(seconds * rate)
    if n_samples <= want:
        return [("Whole", 0, n_samples)]
    if n_samples < 3 * want:
        third = n_samples // 3
        return [("Intro", 0, third), ("Middle", third, 2 * third), ("Outro", 2 * third, n_samples)]
    mid = (n_samples - want) // 2
    return [("Intro", 0, want), ("Middle", mid, mid + want), ("Outro", n_samples - want, n_samples)]


def clip_text(text: str, limit: int) -> str:
    """`text` cut to at most `limit` characters, ending at the last sentence end that fits (hard cut if none)."""
    text = text.strip()
    if len(text) <= limit:
        return text
    head = text[:limit]
    end = max(head.rfind(". "), head.rfind("! "), head.rfind("? "), head.rfind(".\n"))
    if head.endswith((".", "!", "?")):
        end = len(head) - 1
    return head[:end + 1].strip() if end > 0 else head.strip()


def join_sections(parts: list[tuple[str, str]]) -> str:
    if len(parts) == 1:
        return parts[0][1]
    return "\n".join(f"{label}: {text}" for label, text in parts)


def trim_silence(wav: Any, rate: int) -> Any:
    """Drop leading/trailing silence (the model invents sounds in silent tails)."""
    import librosa

    out, _ = librosa.effects.trim(wav, top_db=SILENCE_DB, frame_length=2048, hop_length=512)
    return out if out.size else wav


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
    wav = trim_silence(wav, SAMPLE_RATE)
    parts = []
    for label, a, b in section_windows(wav.size, SAMPLE_RATE, SECTION_SECONDS):
        text = _caption(model, processor, wav[a:b], audio_path)
        parts.append((label, clip_text(text, SECTION_CHARS)))
    return join_sections(parts)


def _caption(model: Any, processor: Any, wav: Any, audio_path: str) -> str:
    import torch

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
