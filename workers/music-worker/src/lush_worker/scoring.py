"""Scoring for LoRA listening renders: pure numpy helpers plus the scorer subprocess client.

Each scorer lives in `SCORER_DIR/<name>/scorer.json` and is a script speaking JSON lines on
stdin/stdout (docs/WORKER_PROTOCOL.md, "Scorer subprocess"). librosa (optional `analysis` extra) is
imported lazily, only by `chroma`.
"""
from __future__ import annotations

import json
import logging
import queue
import subprocess
import threading
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Optional, Sequence

import numpy as np

log = logging.getLogger(__name__)

SCORE_TIMEOUT = 300.0  # seconds per file
STOP_GRACE = 5.0
PROVIDES = ("embedding", "quality")

# Memorization thresholds (chưa hiệu chỉnh): picked by ear before any real data, to be calibrated
# against real eval runs. They only drive a warning badge, never an automatic decision.
MERT_MEMORIZE = 0.95
MELODY_MEMORIZE = 0.80


# ---------- pure helpers ----------
def cosine(a: Sequence[float], b: Sequence[float]) -> float:
    va, vb = np.asarray(a, dtype=np.float64), np.asarray(b, dtype=np.float64)
    denom = float(np.linalg.norm(va) * np.linalg.norm(vb))
    if denom == 0.0:
        return 0.0
    return float(np.dot(va, vb) / denom)


def style_score(render_embs: Sequence[Sequence[float]], train_embs: Sequence[Sequence[float]]) -> float:
    """Cosine between the mean render embedding and the mean training-track embedding (R7).

    Raises ValueError on empty input. Callers treat a non-finite score as missing."""
    if not len(render_embs) or not len(train_embs):
        raise ValueError("no embeddings")
    return cosine(np.mean(np.asarray(render_embs, dtype=np.float64), axis=0),
                  np.mean(np.asarray(train_embs, dtype=np.float64), axis=0))


def best_match(render_emb: Sequence[float], train_embs: Sequence[Sequence[float]],
               names: Sequence[str]) -> tuple[float, str]:
    """Highest cosine between one render and every training track, with that track's name."""
    if not len(train_embs):
        raise ValueError("no embeddings")
    if len(train_embs) != len(names):
        raise ValueError("train_embs and names must be the same length")
    scores = [cosine(render_emb, t) for t in train_embs]
    i = int(np.argmax(scores))
    return scores[i], names[i]


def _check_chroma(chroma_a: np.ndarray, chroma_b: np.ndarray) -> tuple[np.ndarray, np.ndarray]:
    a = np.asarray(chroma_a, dtype=np.float64)
    b = np.asarray(chroma_b, dtype=np.float64)
    if a.ndim != 2 or b.ndim != 2 or a.shape[0] != b.shape[0] or a.shape[1] == 0 or b.shape[1] == 0:
        raise ValueError("chroma must be (bins, frames) arrays with the same bins and at least one frame")
    return a, b


def _clip_similarity(cost: float, steps: int) -> float:
    value = 1.0 - cost / steps
    return float(min(1.0, max(0.0, value))) if np.isfinite(value) else 0.0  # non-finite (silence) -> 0


def _melody_librosa(chroma_a: np.ndarray, chroma_b: np.ndarray) -> float:
    import librosa

    a, b = _check_chroma(chroma_a, chroma_b)
    try:
        with np.errstate(all="ignore"):
            acc, path = librosa.sequence.dtw(X=a, Y=b, metric="cosine")
    except librosa.util.exceptions.ParameterError:  # NaN cost: a silent (all-zero) frame
        return 0.0
    return _clip_similarity(float(acc[-1, -1]), len(path))


def _melody_numpy(chroma_a: np.ndarray, chroma_b: np.ndarray) -> float:
    """Pure-numpy DTW (slow: Python double loop); fallback when librosa is not installed."""
    a, b = _check_chroma(chroma_a, chroma_b)
    na = np.linalg.norm(a, axis=0)
    nb = np.linalg.norm(b, axis=0)
    norm = na[:, None] * nb[None, :]
    sim = (a.T @ b) / np.where(norm == 0.0, 1.0, norm)
    cost = 1.0 - sim  # silent frames have similarity 0 -> distance 1
    n, m = cost.shape
    acc = np.full((n + 1, m + 1), np.inf)
    acc[0, 0] = 0.0
    for i in range(1, n + 1):
        for j in range(1, m + 1):
            acc[i, j] = cost[i - 1, j - 1] + min(acc[i - 1, j], acc[i, j - 1], acc[i - 1, j - 1])
    i, j, steps, total = n, m, 0, 0.0
    while i > 0 and j > 0:  # walk back along the cheapest predecessors
        total += cost[i - 1, j - 1]
        steps += 1
        k = int(np.argmin([acc[i - 1, j - 1], acc[i - 1, j], acc[i, j - 1]]))
        if k == 0:
            i, j = i - 1, j - 1
        elif k == 1:
            i -= 1
        else:
            j -= 1
    return _clip_similarity(total, steps)


def melody_similarity(chroma_a: np.ndarray, chroma_b: np.ndarray) -> float:
    """DTW over cosine frame distance on (12, T) chroma; 1 - cost / len(path), clipped to [0, 1].

    Uses librosa's DTW when importable (fast), else the numpy loop. Non-finite results -> 0.0."""
    try:
        import librosa  # noqa: F401
    except ImportError:
        return _melody_numpy(chroma_a, chroma_b)
    return _melody_librosa(chroma_a, chroma_b)


def chroma(path: str | Path, seconds: float = 60.0) -> np.ndarray:
    """chroma_cqt of the first `seconds` of a file (22050 Hz mono, hop 2048). Needs librosa."""
    import librosa

    y, rate = librosa.load(str(path), sr=22050, mono=True, duration=seconds)
    return librosa.feature.chroma_cqt(y=y, sr=rate, hop_length=2048)


def memorized(train: Optional[float], melody: Optional[float]) -> bool:
    return (train is not None and train >= MERT_MEMORIZE) or (melody is not None and melody >= MELODY_MEMORIZE)


# ---------- discovery ----------
@dataclass(frozen=True)
class ScorerSpec:
    key: str
    name: str
    version: str
    command: list[str]
    provides: str
    directory: Path

    def describe(self) -> dict[str, Any]:
        return {"engine": "scorer", "key": self.key, "name": self.name, "version": self.version,
                "capabilities": [self.provides]}


def load_scorers(scorer_dir: Optional[Path]) -> dict[str, ScorerSpec]:
    if scorer_dir is None or not scorer_dir.is_dir():
        return {}
    specs: dict[str, ScorerSpec] = {}
    for cfg_path in sorted(scorer_dir.glob("*/scorer.json")):
        try:
            raw = json.loads(cfg_path.read_text(encoding="utf-8"))
            if not isinstance(raw, dict):
                raise ValueError("not a JSON object")
            for field in ("key", "name"):
                if not isinstance(raw.get(field), str) or not raw[field].strip():
                    raise ValueError(f"missing or empty '{field}'")
            cmd = raw.get("command")
            if not (isinstance(cmd, list) and cmd and all(isinstance(c, str) and c for c in cmd)):
                raise ValueError("'command' must be a non-empty list of strings")
            if raw.get("provides") not in PROVIDES:
                raise ValueError(f"provides must be one of {list(PROVIDES)}")
            version = raw.get("version", "1")
            if not isinstance(version, str):
                raise ValueError("'version' must be a string")
        except (OSError, ValueError) as e:
            log.warning("scorer %s ignored: %s", cfg_path, e)
            continue
        if raw["key"] in specs:
            log.warning("scorer %s ignored: duplicate key %r", cfg_path, raw["key"])
            continue
        specs[raw["key"]] = ScorerSpec(raw["key"], raw["name"], version, list(cmd), raw["provides"],
                                       cfg_path.parent)
    return specs


# ---------- subprocess client ----------
class ScorerError(Exception):
    """The scorer reported an error for a file, timed out, or died."""


class Scorer:
    def __init__(self, spec: ScorerSpec, timeout: float = SCORE_TIMEOUT):
        self.spec = spec
        self.timeout = timeout
        self._proc: Optional[subprocess.Popen] = None
        self._lines: "queue.Queue[Optional[str]]" = queue.Queue()
        self._timed_out = False

    def start(self) -> None:
        self._timed_out = False
        self._lines = queue.Queue()
        self._proc = subprocess.Popen(
            self.spec.command, cwd=str(self.spec.directory), stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=None, text=True, bufsize=1, encoding="utf-8", errors="replace")
        threading.Thread(target=self._read, args=(self._proc, self._lines),
                         name=f"scorer-{self.spec.key}", daemon=True).start()

    @staticmethod
    def _read(proc: subprocess.Popen, lines: "queue.Queue[Optional[str]]") -> None:
        try:
            for line in proc.stdout:  # type: ignore[union-attr]
                lines.put(line)
        except (OSError, ValueError):
            pass
        finally:
            lines.put(None)

    def score(self, path: str | Path) -> list[float] | float:
        proc = self._proc
        if self._timed_out:
            raise ScorerError("scorer stopped after a timeout")
        if proc is None:
            raise ScorerError("the scorer is not started")
        try:
            proc.stdin.write(json.dumps({"path": str(path)}, ensure_ascii=False) + "\n")  # type: ignore[union-attr]
            proc.stdin.flush()  # type: ignore[union-attr]
        except (OSError, ValueError) as e:
            raise ScorerError(f"cannot write to the scorer: {e}") from e
        deadline = time.monotonic() + self.timeout
        while True:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                # No request id: a late reply could be taken for the next file's answer, so stop it.
                self._timed_out = True
                self.close()
                raise ScorerError(f"no answer within {self.timeout:g}s")
            try:
                line = self._lines.get(timeout=min(0.5, remaining))
            except queue.Empty:
                continue
            if line is None:
                raise ScorerError(f"the scorer exited (code {proc.poll()})")
            line = line.strip()
            if not line:
                continue
            try:
                obj = json.loads(line)
            except ValueError:
                log.debug("scorer stdout (not JSON): %s", line[:200])
                continue
            if not isinstance(obj, dict):
                continue
            if obj.get("error"):
                raise ScorerError(str(obj["error"]))
            if self.spec.provides == "embedding" and isinstance(obj.get("embedding"), list):
                return [float(x) for x in obj["embedding"]]
            if self.spec.provides == "quality" and isinstance(obj.get("quality"), (int, float)):
                return float(obj["quality"])
            raise ScorerError(f"unexpected scorer answer (expected '{self.spec.provides}')")

    def close(self) -> None:
        proc, self._proc = self._proc, None
        if proc is None:
            return
        try:
            proc.stdin.close()  # type: ignore[union-attr]
        except (OSError, ValueError):
            pass
        try:
            proc.wait(STOP_GRACE)
        except subprocess.TimeoutExpired:
            proc.terminate()
            try:
                proc.wait(STOP_GRACE)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait()
        try:
            if proc.stdout:
                proc.stdout.close()
        except (OSError, ValueError):
            pass
