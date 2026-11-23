"""Tempo and key estimates with a confidence. Below the threshold the value is None: no guessing.

- Tempo: librosa beat tracking. Confidence = how much stronger the onset envelope is on the tracked
  beats than on average (a steady pulse scores near 1, noise near 0). Half/double tempo are always
  offered as alternatives because beat trackers often lock to the wrong metrical level.
- Key: Krumhansl-Schmuckler profile correlation on the mean CQT chroma. Confidence = margin of the
  best key over the runner-up, and the best correlation must itself be clear.
Thresholds are starting values chosen on synthetic signals; recalibrate them on reviewed real audio.
"""
from __future__ import annotations

from typing import Optional

import numpy as np
from pydantic import BaseModel

MIN_TEMPO_CONFIDENCE = 0.3
MIN_KEY_CONFIDENCE = 0.05     # margin best - second best correlation
MIN_KEY_CORRELATION = 0.6

PITCHES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]
MAJOR = np.array([6.35, 2.23, 3.48, 2.33, 4.38, 4.09, 2.52, 5.19, 2.39, 3.66, 2.29, 2.88])
MINOR = np.array([6.33, 2.68, 3.52, 5.38, 2.60, 3.53, 2.54, 4.75, 3.98, 2.69, 3.34, 3.17])


class TempoEstimate(BaseModel):
    bpm: Optional[float]
    confidence: float
    alternatives: list[float]
    method: str
    threshold: float


class KeyEstimate(BaseModel):
    key: Optional[str]
    confidence: float
    correlation: float
    method: str
    threshold: float


def _version() -> str:
    import librosa
    return librosa.__version__


def estimate_tempo(mono: np.ndarray, rate: int, min_conf: float = MIN_TEMPO_CONFIDENCE) -> TempoEstimate:
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
    return TempoEstimate(bpm=value, confidence=round(conf, 3), alternatives=alts,
                         method=f"librosa.beat.beat_track {_version()}", threshold=min_conf)


def estimate_key(mono: np.ndarray, rate: int, min_conf: float = MIN_KEY_CONFIDENCE,
                 min_corr: float = MIN_KEY_CORRELATION) -> KeyEstimate:
    import librosa
    y = np.asarray(mono, dtype=np.float32)
    chroma = librosa.feature.chroma_cqt(y=y, sr=rate).mean(axis=1)
    method = f"krumhansl-schmuckler on librosa.chroma_cqt {_version()}"
    if not np.isfinite(chroma).all() or chroma.std() < 1e-6:
        return KeyEstimate(key=None, confidence=0.0, correlation=0.0, method=method, threshold=min_conf)
    scores: list[tuple[float, str]] = []
    for i, tonic in enumerate(PITCHES):
        for profile, mode in ((MAJOR, "major"), (MINOR, "minor")):
            r = float(np.corrcoef(chroma, np.roll(profile, i))[0, 1])
            scores.append((r, f"{tonic} {mode}"))
    scores.sort(reverse=True)
    best, second = scores[0], scores[1]
    margin = best[0] - second[0]
    key = best[1] if margin >= min_conf and best[0] >= min_corr else None
    return KeyEstimate(key=key, confidence=round(margin, 3), correlation=round(best[0], 3),
                       method=method, threshold=min_conf)
