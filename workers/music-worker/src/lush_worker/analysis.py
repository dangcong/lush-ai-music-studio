"""Track analysis for the `analyze_track` job: tempo, key, loudness, clipping, silence.

The tempo / key algorithms are copied from lush-training (D28: the worker imports nothing from it).
Below the confidence threshold a value is None: no guessing.
- Tempo: librosa beat tracking; confidence = how much stronger the onset envelope is on the tracked
  beats than on average. Half/double tempo are always offered as alternatives.
- Key: Krumhansl-Schmuckler profile correlation on the mean CQT chroma; confidence = margin of the
  best key over the runner-up, and the best correlation must itself be clear.
librosa / soundfile / pyloudnorm come from the optional `analysis` extra and are imported lazily.
"""
from __future__ import annotations

import importlib.util
import logging
from pathlib import Path
from typing import Any, Optional

import numpy as np

from .audio import probe_duration

log = logging.getLogger(__name__)

MIN_TEMPO_CONFIDENCE = 0.3
MIN_KEY_CONFIDENCE = 0.05     # margin best - second best correlation
MIN_KEY_CORRELATION = 0.6
ANALYSIS_RATE = 22050
CLIP_LEVEL = 0.999
SILENCE_DBFS = -60.0
FRAME = 2048
HOP = 512

PITCHES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]
MAJOR = np.array([6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88])
MINOR = np.array([6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17])


def analysis_available() -> bool:
    """True when librosa can be imported (the `analysis` extra is installed)."""
    try:
        return importlib.util.find_spec("librosa") is not None
    except (ImportError, ValueError):
        return False


def estimate_tempo(mono: np.ndarray, rate: int, min_conf: float = MIN_TEMPO_CONFIDENCE) -> dict[str, Any]:
    import librosa
    y = np.asarray(mono, dtype=np.float32)
    env = librosa.onset.onset_strength(y=y, sr=rate)
    tempo, beats = librosa.beat.beat_track(onset_envelope=env, sr=rate)
    bpm = float(np.atleast_1d(tempo)[0])
    mean_all = float(env.mean()) if env.size else 0.0
    if beats.size < 4 or mean_all <= 0 or bpm <= 0:
        conf = 0.0
    else:
        ratio = float(env[beats].mean()) / mean_all
        conf = float(np.clip((ratio - 1.0) / 2.0, 0.0, 1.0))
    value = round(bpm, 1) if conf >= min_conf else None
    alts = [round(bpm / 2, 1), round(bpm * 2, 1)] if bpm > 0 else []
    return {"bpm": value, "confidence": round(conf, 3), "alternatives": alts}


def estimate_key(mono: np.ndarray, rate: int, min_conf: float = MIN_KEY_CONFIDENCE,
                 min_corr: float = MIN_KEY_CORRELATION) -> dict[str, Any]:
    import librosa
    y = np.asarray(mono, dtype=np.float32)
    chroma = librosa.feature.chroma_cqt(y=y, sr=rate).mean(axis=1)
    if not np.isfinite(chroma).all() or chroma.std() < 1e-6:
        return {"key": None, "confidence": 0.0}
    scores: list[tuple[float, str]] = []
    for i, tonic in enumerate(PITCHES):
        for profile, mode in ((MAJOR, "major"), (MINOR, "minor")):
            r = float(np.corrcoef(chroma, np.roll(profile, i))[0, 1])
            scores.append((r, f"{tonic} {mode}"))
    scores.sort(reverse=True)
    best, second = scores[0], scores[1]
    margin = best[0] - second[0]
    key = best[1] if margin >= min_conf and best[0] >= min_corr else None
    return {"key": key, "confidence": round(margin, 3)}


def _silence_ratio(y: np.ndarray) -> float:
    if y.size < FRAME:
        return 1.0 if y.size == 0 or float(np.sqrt(np.mean(y ** 2))) < 10 ** (SILENCE_DBFS / 20) else 0.0
    starts = np.arange(0, y.size - FRAME + 1, HOP)
    sq = np.cumsum(np.concatenate([[0.0], y.astype(np.float64) ** 2]))
    rms = np.sqrt((sq[starts + FRAME] - sq[starts]) / FRAME)
    return float(np.mean(rms < 10 ** (SILENCE_DBFS / 20)))


def _loudness_lufs(native: np.ndarray, rate: int) -> Optional[float]:
    """Integrated loudness of the decoded file at its native rate. `native` is (channels, samples).
    None when pyloudnorm is missing or the file is too short; logs a warning when measuring fails."""
    try:
        import pyloudnorm
    except ImportError:
        log.warning("loudness skipped: pyloudnorm is not installed")
        return None
    if native.shape[1] < rate * 0.4:  # BS.1770 needs at least one 400 ms block
        return None
    try:
        value = float(pyloudnorm.Meter(rate).integrated_loudness(native.T.astype(np.float64)))
    except Exception as e:
        log.warning("loudness could not be computed: %s", e)
        return None
    return round(value, 1) if np.isfinite(value) else None


def analyze_file(path: Path) -> dict[str, Any]:
    import librosa
    # Decode once at the native rate with all channels; everything else derives from this array.
    native, native_rate = librosa.load(str(path), sr=None, mono=False)
    native = np.atleast_2d(native)
    y = librosa.to_mono(native)
    if native_rate != ANALYSIS_RATE:
        y = librosa.resample(y, orig_sr=native_rate, target_sr=ANALYSIS_RATE)
    duration = probe_duration(path)
    if duration is None:
        duration = native.shape[1] / native_rate
    return {
        "duration_s": round(float(duration), 2),
        "tempo": estimate_tempo(y, ANALYSIS_RATE),
        "key": estimate_key(y, ANALYSIS_RATE),
        "loudness_lufs": _loudness_lufs(native, native_rate),
        "clipping": bool(native.size and float(np.max(np.abs(native))) >= CLIP_LEVEL),
        "silence_ratio": round(_silence_ratio(y), 3),
    }
