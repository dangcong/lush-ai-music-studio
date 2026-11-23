"""`caption_tracks` jobs: describe catalog tracks with a captioner model run as a subprocess.

Each captioner lives in `CAPTIONER_DIR/<name>/captioner.json` and is a script speaking JSON lines on
stdin/stdout (docs/WORKER_PROTOCOL.md, "Captioner subprocess"). The script is started once per job
so the model loads once and is released when the job ends.
"""
from __future__ import annotations

import json
import logging
import queue
import re
import subprocess
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Optional

import httpx

from .api_client import ApiError
from .engines.base import EngineError, LeaseLost

log = logging.getLogger(__name__)

JOB_TYPE = "caption_tracks"
TRACK_TIMEOUT = 600.0
STOP_GRACE = 5.0
MODES = ("json", "text")
POST_ATTEMPTS = 3

DEFAULT_JSON_PROMPT = (
    'Listen to this track and answer with JSON only: {"mood": "...", "instruments": "...", '
    '"vocals": "Instrumental or describe any voice", "caption": "one English paragraph under 80 words '
    'describing genre, instruments, mood and texture; do not state BPM or key"}')

INSTRUMENT_TERMS = [
    "piano", "electric piano", "Rhodes", "Wurlitzer", "organ", "keys", "synth", "synthesizer", "pad",
    "guitar", "acoustic guitar", "electric guitar", "bass", "upright bass", "double bass", "drums",
    "brushed drums", "drum machine", "percussion", "strings", "violin", "cello", "harp", "flute",
    "saxophone", "trumpet", "clarinet", "horns", "bells", "marimba", "vibraphone", "harmonica",
    "vinyl", "tape", "sub bass", "808", "hi-hats", "shaker", "ukulele", "banjo",
]
MOOD_TERMS = [
    "mellow", "nostalgic", "calm", "dreamy", "melancholic", "melancholy", "warm", "chill", "relaxed",
    "peaceful", "soothing", "cozy", "gentle", "soft", "sad", "happy", "uplifting", "dark", "moody",
    "atmospheric", "ambient", "romantic", "hopeful", "wistful", "lonely", "tender", "serene", "playful",
    "energetic", "tense", "mysterious", "sentimental", "hazy", "smooth", "laid-back", "bittersweet",
    "introspective", "contemplative", "cheerful", "rainy",
]
# Must stay identical to VOICE_TERMS in apps/api/src/routes/catalogs.rs.
VOICE_TERMS = ["vocal", "vocals", "voice", "singing", "singer", "sung", "hum", "humming", "chant",
               "spoken", "speech", "rap", "choir", "lyrics"]


def _term_regex(terms: list[str]) -> re.Pattern[str]:
    alternation = "|".join(re.escape(t) for t in sorted(terms, key=len, reverse=True))
    return re.compile(rf"(?<![A-Za-z0-9])(?:{alternation})(?![A-Za-z0-9])", re.IGNORECASE)


_INSTRUMENT_RE = _term_regex(INSTRUMENT_TERMS)
_MOOD_RE = _term_regex(MOOD_TERMS)
_VOICE_RE = _term_regex(VOICE_TERMS)
_INSTRUMENTAL_RE = re.compile(r"(?<![A-Za-z0-9])instrumental(?![A-Za-z0-9])", re.IGNORECASE)
_CANONICAL = {t.lower(): t for t in INSTRUMENT_TERMS + MOOD_TERMS}


def _find_terms(pattern: re.Pattern[str], text: str) -> str:
    seen: list[str] = []
    for m in pattern.finditer(text):
        term = _CANONICAL[m.group(0).lower()]
        if term not in seen:
            seen.append(term)
    return ", ".join(seen)


def extract_fields(caption: str) -> dict[str, str]:
    if _VOICE_RE.search(caption):
        vocals = "Vocals"
    elif _INSTRUMENTAL_RE.search(caption):
        vocals = "Instrumental"
    else:
        vocals = ""
    return {"mood": _find_terms(_MOOD_RE, caption), "instruments": _find_terms(_INSTRUMENT_RE, caption),
            "vocals": vocals, "caption": caption.strip()}


def _as_text(value: Any) -> str:
    if value is None:
        return ""
    if isinstance(value, (list, tuple)):
        return ", ".join(str(v).strip() for v in value if str(v).strip())
    return str(value).strip()


def parse_output(text: str, mode: str) -> Optional[dict[str, str]]:
    """Captioner output -> {mood, instruments, vocals, caption}, or None when unusable."""
    if mode == "text":
        caption = (text or "").strip()
        return extract_fields(caption) if caption else None
    decoder = json.JSONDecoder()
    start = (text or "").find("{")
    while start != -1:
        try:
            obj, _ = decoder.raw_decode(text, start)
        except ValueError:
            start = text.find("{", start + 1)
            continue
        if not isinstance(obj, dict):
            start = text.find("{", start + 1)
            continue
        fields = {k: _as_text(obj.get(k)) for k in ("mood", "instruments", "vocals", "caption")}
        if not fields["caption"]:
            return None
        derived = extract_fields(fields["caption"])
        for k in ("mood", "instruments", "vocals"):
            fields[k] = fields[k] or derived[k]
        return fields
    return None


# ---------- discovery ----------
@dataclass(frozen=True)
class CaptionerSpec:
    key: str
    name: str
    version: str
    python: str
    script: Path
    mode: str
    directory: Path
    prompt: Optional[str] = None

    def describe(self) -> dict[str, Any]:
        return {"engine": "captioner", "key": self.key, "name": self.name, "version": self.version,
                "capabilities": ["caption"]}


def _resolve(base: Path, value: str) -> Path:
    p = Path(value).expanduser()
    return p if p.is_absolute() else (base / p)


def discover_captioners(root: Optional[Path]) -> list[CaptionerSpec]:
    if root is None or not root.is_dir():
        return []
    specs: list[CaptionerSpec] = []
    for cfg_path in sorted(root.glob("*/captioner.json")):
        base = cfg_path.parent
        try:
            raw = json.loads(cfg_path.read_text(encoding="utf-8"))
            if not isinstance(raw, dict):
                raise ValueError("not a JSON object")
            for field in ("key", "name", "version", "python", "script", "mode"):
                if not isinstance(raw.get(field), str) or not raw[field].strip():
                    raise ValueError(f"missing or empty '{field}'")
            if raw["mode"] not in MODES:
                raise ValueError(f"mode must be one of {list(MODES)}")
            prompt = raw.get("prompt")
            if prompt is not None and not isinstance(prompt, str):
                raise ValueError("'prompt' must be a string")
            script = _resolve(base, raw["script"])
            if not script.is_file():
                raise ValueError(f"script not found: {script}")
            py = _resolve(base, raw["python"])
            python = str(py) if py.exists() else raw["python"]  # bare command such as "python3"
        except (OSError, ValueError) as e:
            log.warning("captioner %s ignored: %s", cfg_path, e)
            continue
        if any(s.key == raw["key"] for s in specs):
            log.warning("captioner %s ignored: duplicate key %r", cfg_path, raw["key"])
            continue
        specs.append(CaptionerSpec(raw["key"], raw["name"], raw["version"], python, script, raw["mode"],
                                   base, prompt or None))
    return specs


# ---------- subprocess ----------
class _ProcessDied(Exception):
    pass


class _ProcessTimeout(Exception):
    pass


class _CaptionerProcess:
    def __init__(self, spec: CaptionerSpec):
        self.spec = spec
        self.lines: "queue.Queue[Optional[str]]" = queue.Queue()
        self.proc = subprocess.Popen(
            [spec.python, str(spec.script)], cwd=str(spec.directory), stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=None, text=True, bufsize=1, encoding="utf-8", errors="replace")
        threading.Thread(target=self._read, name=f"captioner-{spec.key}", daemon=True).start()

    def _read(self) -> None:
        try:
            for line in self.proc.stdout:  # type: ignore[union-attr]
                self.lines.put(line)
        except (OSError, ValueError):
            pass
        finally:
            self.lines.put(None)

    def request(self, ctx: Any, track_id: str, audio: str, prompt: Optional[str], timeout: float) -> dict[str, Any]:
        msg = json.dumps({"id": track_id, "audio": audio, "prompt": prompt}, ensure_ascii=False)
        try:
            self.proc.stdin.write(msg + "\n")  # type: ignore[union-attr]
            self.proc.stdin.flush()  # type: ignore[union-attr]
        except (OSError, ValueError) as e:
            raise _ProcessDied(f"cannot write to the captioner: {e}") from e
        deadline = time.monotonic() + timeout
        while True:
            ctx.raise_if_aborted()
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise _ProcessTimeout(f"no answer within {timeout:g}s")
            try:
                line = self.lines.get(timeout=min(0.5, remaining))
            except queue.Empty:
                continue
            if line is None:
                code = self.proc.poll()
                raise _ProcessDied(f"the captioner exited (code {code})")
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except ValueError:
                log.debug("captioner stdout (not JSON): %s", line[:200])
                continue
            if isinstance(obj, dict) and str(obj.get("id")) == track_id:
                return obj

    def kill(self) -> None:
        """Terminate, then kill after STOP_GRACE seconds."""
        if self.proc.poll() is None:
            self.proc.terminate()
            try:
                self.proc.wait(STOP_GRACE)
            except subprocess.TimeoutExpired:
                self.proc.kill()
                self.proc.wait()
        self._close_pipes()

    def finish(self) -> None:
        """Normal end: close stdin and let the script exit on its own."""
        try:
            self.proc.stdin.close()  # type: ignore[union-attr]
        except (OSError, ValueError):
            pass
        try:
            self.proc.wait(STOP_GRACE)
        except subprocess.TimeoutExpired:
            pass
        self.kill()

    def _close_pipes(self) -> None:
        for pipe in (self.proc.stdin, self.proc.stdout):
            try:
                if pipe:
                    pipe.close()
            except (OSError, ValueError):
                pass


# ---------- job ----------
def _post(api: Any, ctx: Any, track_id: str, fields: Optional[dict[str, str]], source_ref: str,
          error: Optional[str] = None) -> None:
    for attempt in range(1, POST_ATTEMPTS + 1):
        try:
            api.post_caption(ctx.job_id, ctx.lease_token, track_id, fields, source_ref=source_ref, error=error)
            return
        except ApiError as e:
            if e.lease_lost:
                raise LeaseLost(f"job {ctx.job_id}: lease lost") from e
            if not e.transient or attempt == POST_ATTEMPTS:
                raise
        except httpx.TransportError:
            if attempt == POST_ATTEMPTS:
                raise
        time.sleep(attempt)


def run_caption_job(ctx: Any, api: Any, spec: CaptionerSpec, payload: dict[str, Any],
                    track_timeout: float = TRACK_TIMEOUT) -> dict[str, Any]:
    tracks = payload.get("tracks") or []
    inputs = payload.get("inputs") or {}
    source_ref = f"{spec.key}@{spec.version}"
    prompt = spec.prompt or (DEFAULT_JSON_PROMPT if spec.mode == "json" else None)
    done, failed = 0, []
    proc: Optional[_CaptionerProcess] = None
    restarts_left = 1
    total = max(len(tracks), 1)

    def ask(track_id: str, audio: str) -> Optional[dict[str, Any]]:
        nonlocal proc
        if proc is None:
            proc = _CaptionerProcess(spec)
        try:
            return proc.request(ctx, track_id, audio, prompt, track_timeout)
        except (_ProcessDied, _ProcessTimeout) as e:
            log.warning("job %s: captioner %s failed on %s: %s", ctx.job_id, spec.key, track_id, e)
            proc.kill()
            proc = None
            return {"error": str(e), "_died": True}

    try:
        for i, tr in enumerate(tracks):
            ctx.raise_if_aborted()
            track_id = str(tr["catalog_track_id"])
            info = inputs.get(tr.get("input")) or {}
            audio = info.get("path")
            error: Optional[str] = None
            fields: Optional[dict[str, str]] = None
            if not audio:
                error = "the track has no audio input"
            else:
                raw_text = ""
                for attempt in (1, 2):
                    res = ask(track_id, audio)
                    if res is None or res.get("_died"):
                        error = res["error"] if res else "no answer"
                        if restarts_left <= 0:
                            raise EngineError(f"captioner {spec.key} keeps crashing: {error}")
                        restarts_left -= 1
                        break
                    if res.get("error"):
                        error = str(res["error"])
                        break
                    raw_text = str(res.get("text") or "")
                    fields = parse_output(raw_text, spec.mode)
                    if fields:
                        break
                if fields is None and error is None:
                    if raw_text.strip():
                        fields = extract_fields(raw_text)
                    else:
                        error = "the captioner returned an empty answer"
            if fields is not None:
                _post(api, ctx, track_id, fields, source_ref)
                done += 1
            else:
                _post(api, ctx, track_id, None, source_ref, error=error or "unknown error")
                failed.append(track_id)
            ctx.report((i + 1) / total, f"captioned {i + 1}/{len(tracks)}", stage="running", phase="caption")
    except BaseException:
        if proc is not None:
            proc.kill()
        raise
    if proc is not None:
        proc.finish()
    return {"done": done, "failed": failed}
