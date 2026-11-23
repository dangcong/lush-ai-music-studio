"""Tempo and key estimates carry a confidence; below the threshold the value is None, never a guess."""
import numpy as np
import pytest

pytest.importorskip("librosa")

from conftest import dataset_cfg  # noqa: E402

from lush_training import analysis, captions, dataset  # noqa: E402
from lush_training.util import read_json  # noqa: E402

RATE = 22050
NOTE = {"C": 261.63, "D": 293.66, "E": 329.63, "F": 349.23, "G": 392.00, "A": 440.00, "B": 493.88}


def click_track(bpm: float, seconds: float = 30.0) -> np.ndarray:
    n = int(seconds * RATE)
    out = np.zeros(n, dtype=np.float32)
    burst = (np.random.default_rng(0).standard_normal(400) * np.exp(-np.arange(400) / 60)).astype(np.float32)
    step = 60.0 / bpm
    t = 0.0
    while t * RATE + 400 < n:
        i = int(t * RATE)
        out[i:i + 400] += burst
        t += step
    return out


def noise(seconds: float = 30.0) -> np.ndarray:
    return (0.2 * np.random.default_rng(1).standard_normal(int(seconds * RATE))).astype(np.float32)


def triads(chords: list[list[str]], seconds_each: float = 2.0, repeats: int = 4) -> np.ndarray:
    t = np.arange(int(seconds_each * RATE)) / RATE
    parts = []
    for _ in range(repeats):
        for chord in chords:
            tone = sum(np.sin(2 * np.pi * NOTE[n] * t) + 0.5 * np.sin(2 * np.pi * NOTE[n] / 2 * t) for n in chord)
            parts.append((0.2 * tone).astype(np.float32))
    return np.concatenate(parts)


def test_tempo_click_track():
    est = analysis.estimate_tempo(click_track(90), RATE)
    assert est.bpm is not None and abs(est.bpm - 90) <= 1.5
    assert any(abs(a - 45) <= 1.5 for a in est.alternatives)
    assert any(abs(a - 180) <= 3 for a in est.alternatives)
    assert est.method.startswith("librosa")


def test_tempo_noise_unknown():
    est = analysis.estimate_tempo(noise(), RATE)
    assert est.bpm is None and est.confidence < analysis.MIN_TEMPO_CONFIDENCE


def test_key_c_major_progression():
    est = analysis.estimate_key(triads([["C", "E", "G"], ["F", "A", "C"], ["G", "B", "D"], ["C", "E", "G"]]), RATE)
    assert est.key == "C major"


def test_key_noise_unknown():
    est = analysis.estimate_key(noise(), RATE)
    assert est.key is None


def test_init_work_records_key_evidence(project, tmp_path):
    from conftest import write_audio
    music = tmp_path / "keys"
    chords = [["C", "E", "G"], ["F", "A", "C"], ["G", "B", "D"], ["C", "E", "G"]]
    for i in range(2):
        data = triads(chords)
        write_audio(music / f"song{i}.wav", np.stack([data, data], axis=1), rate=RATE)
    cfg = dataset_cfg(music, segment={"seconds": 240.0, "min_seconds": 3.0, "max_silence_ratio": 0.5},
                      split={"validation": 0.0, "test": 0.0, "seed": 1})
    dataset.scan(cfg, project)
    dataset.build(cfg, project, "v1")
    vdir = dataset.version_dir(project, "testset", "v1")
    captions.init_work(vdir)
    manifest, _ = dataset.load_manifest(vdir)
    sid = manifest["segments"][0]["segment_id"]
    cap = read_json(captions.work_path(vdir, sid))
    key_ev = [e for e in cap["evidence"] if e["field"] == "key"]
    assert key_ev and key_ev[0]["source"] == "tool" and key_ev[0]["value"] == "C major"
    assert cap["metadata"]["key"] is None          # evidence only; a human decides
    a = read_json(captions.analysis_path(vdir, sid))
    assert a["estimates"]["key"]["key"] == "C major" and "confidence" in a["estimates"]["tempo"]
