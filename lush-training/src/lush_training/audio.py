"""Decode audio and measure it. Measurements are evidence, not judgements: thresholds live in the
dataset config, and nothing here decides that a file "has vocals" or "sounds good"."""
from __future__ import annotations

from pathlib import Path
from typing import Any

import numpy as np
import soundfile as sf

from .util import LushTrainingError

AUDIO_EXTENSIONS = (".wav", ".flac", ".mp3")
FRAME_SECONDS = 0.1
SILENCE_DBFS = -60.0
CLIP_LEVEL = 0.999
FP_BANDS = 16
FP_HOP_SECONDS = 0.5


def decode(path: Path) -> tuple[np.ndarray, int]:
    """(samples[frames, channels] float32, sample_rate). Raises LushTrainingError when undecodable."""
    try:
        data, rate = sf.read(str(path), dtype="float32", always_2d=True)
    except Exception as e:  # libsndfile raises several types for corrupt files
        raise LushTrainingError(f"không giải mã được {path}: {e}") from e
    if data.shape[0] == 0:
        raise LushTrainingError(f"{path}: không có mẫu âm thanh nào")
    return data, int(rate)


def dbfs(value: float) -> float:
    return float(20.0 * np.log10(max(value, 1e-10)))


def frame_rms(mono: np.ndarray, rate: int, seconds: float = FRAME_SECONDS) -> np.ndarray:
    size = max(1, int(rate * seconds))
    n = len(mono) // size
    if n == 0:
        return np.array([float(np.sqrt(np.mean(mono ** 2)))])
    frames = mono[: n * size].reshape(n, size)
    return np.sqrt(np.mean(frames ** 2, axis=1))


def measure(data: np.ndarray, rate: int) -> dict[str, Any]:
    """Technical facts about a decoded signal."""
    mono = data.mean(axis=1)
    rms = frame_rms(mono, rate)
    rms_db = 20.0 * np.log10(np.maximum(rms, 1e-10))
    peak = float(np.max(np.abs(data)))
    clipped = np.abs(data) >= CLIP_LEVEL
    # A clipped run = at least 3 consecutive samples at full scale on one channel.
    runs = clipped[:-2] & clipped[1:-1] & clipped[2:] if len(data) > 2 else clipped[:0]
    loud = rms_db[rms_db > SILENCE_DBFS]
    return {
        "duration": round(len(data) / rate, 3),
        "sample_rate": rate,
        "channels": int(data.shape[1]),
        "peak_dbfs": round(dbfs(peak), 2),
        "rms_dbfs": round(dbfs(float(np.sqrt(np.mean(mono ** 2)))), 2),
        "silence_ratio": round(float(np.mean(rms_db <= SILENCE_DBFS)), 4),
        "clipped_sample_ratio": round(float(np.mean(runs)) if runs.size else 0.0, 6),
        "dc_offset": round(float(np.mean(mono)), 5),
        # Spread of loudness between quiet and loud passages (non-silent frames only).
        "dynamic_range_db": round(float(np.percentile(loud, 95) - np.percentile(loud, 10)), 2) if loud.size > 1 else None,
    }


def spectral_summary(data: np.ndarray, rate: int) -> dict[str, Any]:
    """Coarse spectrum facts used as caption evidence (texture / brightness), not as a judgement."""
    mono = data.mean(axis=1)
    size = 4096
    n = len(mono) // size
    if n == 0:
        return {"spectral_centroid_hz": None, "band_energy": None}
    frames = mono[: n * size].reshape(n, size) * np.hanning(size)
    power = np.mean(np.abs(np.fft.rfft(frames, axis=1)) ** 2, axis=0)
    freqs = np.fft.rfftfreq(size, 1.0 / rate)
    total = float(power.sum())
    if total <= 0:
        return {"spectral_centroid_hz": None, "band_energy": None}
    def band(lo: float, hi: float) -> float:
        return round(float(power[(freqs >= lo) & (freqs < hi)].sum() / total), 4)
    return {"spectral_centroid_hz": round(float((freqs * power).sum() / total), 1),
            "band_energy": {"low_<250hz": band(0, 250), "mid_250-4000hz": band(250, 4000),
                            "high_>=4000hz": band(4000, rate)}}


def fingerprint(data: np.ndarray, rate: int) -> np.ndarray:
    """Coarse log band-energy envelope [time, FP_BANDS] at ~2 rows per second, 100 Hz - 4 kHz.

    Gain-invariant and tolerant to lossy re-encoding; used only to flag near-duplicate CANDIDATES.
    It does not recognise a remix, a different take or a transposed version.
    """
    mono = data.mean(axis=1)
    size = max(256, int(rate * FP_HOP_SECONDS))
    n = len(mono) // size
    if n < 2:
        return np.zeros((0, FP_BANDS), dtype=np.float32)
    spec = np.abs(np.fft.rfft(mono[: n * size].reshape(n, size) * np.hanning(size), axis=1)) ** 2
    freqs = np.fft.rfftfreq(size, 1.0 / rate)
    edges = np.geomspace(100.0, min(4000.0, rate / 2 * 0.95), FP_BANDS + 1)
    bands = np.stack([spec[:, (freqs >= lo) & (freqs < hi)].sum(axis=1) for lo, hi in zip(edges[:-1], edges[1:])], axis=1)
    logb = np.log10(bands + 1e-12)
    logb -= logb.mean()  # removes overall gain
    return logb.astype(np.float32)


def fingerprint_similarity(a: np.ndarray, b: np.ndarray) -> float:
    """Cosine similarity of two fingerprints over their common length (0 when not comparable)."""
    n = min(len(a), len(b))
    if n < 4:
        return 0.0
    x, y = a[:n].ravel(), b[:n].ravel()
    denom = float(np.linalg.norm(x) * np.linalg.norm(y))
    return float(np.dot(x, y) / denom) if denom > 0 else 0.0


def write_segment(path: Path, data: np.ndarray, rate: int) -> None:
    """Write a segment as 24-bit WAV. The original file is never modified."""
    path.parent.mkdir(parents=True, exist_ok=True)
    sf.write(str(path), data, rate, subtype="PCM_24", format="WAV")
