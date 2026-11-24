"""Tempo / key / loudness analysis (librosa needed; runner wiring is tested in test_runner.py)."""
from __future__ import annotations

import wave

import numpy as np
import pytest

pytest.importorskip("librosa")

from lush_worker.analysis import analyze_file, estimate_key, estimate_tempo, measure_tempo_key  # noqa: E402

SR = 22050


def click_track(bpm: float, seconds: float) -> np.ndarray:
    y = np.zeros(int(SR * seconds), dtype=np.float32)
    step = 60.0 / bpm
    n = 0
    while n * step < seconds - 0.1:
        i = int(n * step * SR)
        burst = np.hanning(220) * np.sin(2 * np.pi * 1500 * np.arange(220) / SR)
        y[i:i + 220] += burst.astype(np.float32)
        n += 1
    return y


def chord(freqs: list[float], seconds: float) -> np.ndarray:
    t = np.arange(int(SR * seconds)) / SR
    y = sum(np.sin(2 * np.pi * f * t) + 0.5 * np.sin(2 * np.pi * 2 * f * t) for f in freqs)
    env = np.minimum(1.0, np.minimum(t, seconds - t) * 20)
    return (y * env / len(freqs) / 2).astype(np.float32)


def write_wav(path, y: np.ndarray) -> None:
    pcm = (np.clip(y, -1, 1) * 32767).astype("<i2")
    with wave.open(str(path), "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())


def test_tempo_of_click_track():
    t = estimate_tempo(click_track(120, 20), SR)
    # librosa's tempo lattice at hop 512 / 22.05 kHz gives 117.5 for a true 120, so allow +-2.5%.
    assert any(117 <= v <= 123 for v in [t["bpm"], *t["alternatives"]] if v is not None)
    assert len(t["alternatives"]) == 2


def test_key_of_d_minor_chord_loop():
    dm, gm, a = [146.83, 174.61, 220.0], [196.0, 233.08, 293.66], [220.0, 277.18, 329.63]
    y = np.concatenate([chord(c, 2.0) for c in [dm, gm, a, dm] * 3])
    k = estimate_key(y, SR)
    # Either the right key or "unknown" (below threshold); never a wrong key.
    assert k["key"] in ("D minor", None)


def test_silence_returns_unknown(tmp_path):
    y = np.zeros(SR * 5, dtype=np.float32)
    assert estimate_tempo(y, SR)["bpm"] is None
    assert estimate_key(y, SR)["key"] is None
    p = tmp_path / "s.wav"
    write_wav(p, y)
    r = analyze_file(p)
    assert r["silence_ratio"] > 0.9 and r["tempo"]["bpm"] is None and r["key"]["key"] is None
    assert r["clipping"] is False


def test_analyze_file_schema_and_clipping(tmp_path):
    y = click_track(120, 8) * 3
    p = tmp_path / "c.wav"
    write_wav(p, y)
    r = analyze_file(p)
    assert set(r) == {"duration_s", "tempo", "key", "loudness_lufs", "clipping", "silence_ratio"}
    assert set(r["tempo"]) == {"bpm", "confidence", "alternatives"}
    assert set(r["key"]) == {"key", "confidence"}
    assert r["duration_s"] == pytest.approx(8.0, abs=0.05)
    assert r["clipping"] is True


def write_stereo_wav(path, left: np.ndarray, right: np.ndarray) -> None:
    pcm = (np.clip(np.stack([left, right], axis=1), -1, 1) * 32767).astype("<i2")
    with wave.open(str(path), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(SR)
        w.writeframes(pcm.tobytes())


def test_clipping_in_one_stereo_channel(tmp_path):
    left = click_track(120, 6) * 3          # clips
    right = np.zeros_like(left)
    p = tmp_path / "st.wav"
    write_stereo_wav(p, left, right)
    assert analyze_file(p)["clipping"] is True


def test_loudness_does_not_depend_on_soundfile(tmp_path, monkeypatch):
    pytest.importorskip("pyloudnorm")
    import soundfile

    def boom(*a, **k):
        raise RuntimeError("soundfile unavailable")
    monkeypatch.setattr(soundfile, "read", boom)
    t = np.arange(SR * 3) / SR
    p = tmp_path / "tone.wav"
    write_wav(p, (0.3 * np.sin(2 * np.pi * 440 * t)).astype(np.float32))
    r = analyze_file(p)
    assert r["loudness_lufs"] is not None and -40 < r["loudness_lufs"] < 0


def test_measure_tempo_key_reads_only_the_first_seconds(tmp_path):
    # 30 s of clicks then 30 s of silence: reading the first 20 s still finds the tempo.
    y = np.concatenate([click_track(120, 30), np.zeros(SR * 30, dtype=np.float32)])
    write_wav(tmp_path / "clicks.wav", y)
    m = measure_tempo_key(tmp_path / "clicks.wav", seconds=20)
    assert set(m) == {"tempo", "key"}
    t = m["tempo"]
    assert any(117 <= v <= 123 for v in [t["bpm"], *t["alternatives"]] if v is not None)
