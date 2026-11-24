"""ACE-Step Captioner subprocess for the Lush worker (docs/WORKER_PROTOCOL.md, "Captioner subprocess").

stdin : one JSON line per track  {"id", "audio", "prompt"}   (prompt may be null -> DEFAULT_PROMPT)
stdout: one JSON line per answer {"id", "text"} or {"id", "error"}; stdout carries ONLY protocol lines
stderr: all logs
Exits 0 when stdin closes. The model loads once at start-up.

ACE-Step/acestep-captioner is the labelling model of ACE-Step 1.5 training data, a fine-tune of
Qwen2.5-Omni-7B ("usage is the same as Qwen2.5-Omni-7B"); it answers in free text, so captioner.json
uses "mode": "text". Only the thinker (text output) is used; the talker is never loaded. License: MIT
(model card). See README.md for what was self-tested on rtx6000.
"""
from __future__ import annotations

import json
import os
import sys
from typing import IO, Any, Optional

MODEL_DIR = os.environ.get("ACESTEP_CAPTIONER_MODEL_DIR", "./weights/acestep-captioner")
MAX_NEW_TOKENS = int(os.environ.get("ACESTEP_CAPTIONER_MAX_NEW_TOKENS", "1024"))
SAMPLE_RATE = 16000  # Qwen2.5-Omni audio encoder
DEFAULT_PROMPT = "*Task* Describe this audio in detail"  # prompt format from the model card


def log(msg: str) -> None:
    print(f"[acestep-captioner] {msg}", file=sys.stderr, flush=True)


# ---------- model part: heavy imports stay inside ----------
def load_model() -> Any:
    """Return an opaque handle (model, processor)."""
    if os.environ.get("ACESTEP_CAPTIONER_FAKE_GENERATE") == "1":  # test hook
        print("junk from print at load", flush=True)
        return "FAKE"
    import torch  # noqa: F401  (fail early if missing)
    from transformers import Qwen2_5OmniProcessor, Qwen2_5OmniThinkerForConditionalGeneration

    processor = Qwen2_5OmniProcessor.from_pretrained(MODEL_DIR)
    model = Qwen2_5OmniThinkerForConditionalGeneration.from_pretrained(
        MODEL_DIR, torch_dtype="auto", device_map="cuda:0").eval()
    return model, processor


def generate(handle: Any, audio_path: str, prompt: str) -> str:
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
    conversation = [{"role": "user", "content": [
        {"type": "text", "text": prompt},
        {"type": "audio", "audio": audio_path},
    ]}]
    text = processor.apply_chat_template(conversation, add_generation_prompt=True, tokenize=False)
    inputs = processor(text=text, audio=[wav], return_tensors="pt", padding=True).to(model.device)
    for k, v in inputs.items():
        if torch.is_tensor(v) and v.is_floating_point():
            inputs[k] = v.to(model.dtype)
    with torch.no_grad():
        out = model.generate(**inputs, max_new_tokens=MAX_NEW_TOKENS)
    return processor.batch_decode(out[:, inputs["input_ids"].shape[1]:], skip_special_tokens=True)[0].strip()


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
