"""Scoring helpers (pure numpy) and the scorer subprocess protocol (real fake_scorer.py)."""
from __future__ import annotations

import json
import sys
from pathlib import Path

import numpy as np
import pytest

from lush_worker.scoring import (MELODY_MEMORIZE, MERT_MEMORIZE, Scorer, ScorerError, ScorerSpec, best_match,
                                 cosine, load_scorers, melody_similarity, memorized, style_score)

FAKE = Path(__file__).parent / "fake_scorer.py"


def spec_for(tmp_path, provides="embedding") -> ScorerSpec:
    return ScorerSpec("fake-scorer", "Fake scorer", "1", [sys.executable, str(FAKE)], provides, tmp_path)


def one_hot_chroma(pitches: list[int]) -> np.ndarray:
    m = np.zeros((12, len(pitches)))
    for t, p in enumerate(pitches):
        m[p, t] = 1.0
    return m


def test_cosine_and_style_score():
    assert cosine([1, 0], [1, 0]) == pytest.approx(1.0, abs=1e-6)
    assert cosine([1, 0], [0, 1]) == pytest.approx(0.0, abs=1e-6)
    assert cosine([1, 1], [1, 0]) == pytest.approx(2 ** -0.5, abs=1e-6)
    assert cosine([0, 0], [1, 0]) == 0.0
    # mean(render) = [1, 1], mean(train) = [2, 0] -> cos 45 degrees
    assert style_score([[1, 0], [1, 2]], [[2, 0], [2, 0]]) == pytest.approx(2 ** -0.5, abs=1e-6)


def test_best_match_returns_name():
    train = [[1, 0], [0, 1], [1, 1]]
    score, name = best_match([0.0, 2.0], train, ["a.wav", "b.wav", "c.wav"])
    assert name == "b.wav" and score == pytest.approx(1.0, abs=1e-6)
    with pytest.raises(ValueError):
        best_match([1, 0], [], [])


def test_melody_similarity_identical_is_one():
    c = one_hot_chroma([0, 4, 7, 4, 0, 2, 5, 9])
    assert melody_similarity(c, c) == pytest.approx(1.0, abs=1e-6)


def test_melody_similarity_unrelated_is_lower():
    a = one_hot_chroma([0, 4, 7, 4, 0, 2, 5, 9])
    b = one_hot_chroma([1, 3, 6, 8, 10, 11, 1, 3])
    same = melody_similarity(a, a)
    other = melody_similarity(a, b)
    assert 0.0 <= other < same
    assert other == pytest.approx(0.0, abs=1e-6)  # orthogonal chroma frames


def test_memorized_thresholds():
    assert (MERT_MEMORIZE, MELODY_MEMORIZE) == (0.95, 0.80)
    assert memorized(0.95, None) is True
    assert memorized(0.9499, None) is False
    assert memorized(None, 0.80) is True
    assert memorized(None, 0.7999) is False
    assert memorized(0.5, 0.9) is True
    assert memorized(None, None) is False


def test_scorer_subprocess_roundtrip(tmp_path):
    s = Scorer(spec_for(tmp_path))
    s.start()
    try:
        assert s.score("a.wav") == [1.0, 2.0, 3.0]
        assert s.score("b.wav") == [1.0, 2.0, 3.0]  # same process answers again
    finally:
        s.close()


def test_scorer_quality_provides(tmp_path):
    s = Scorer(spec_for(tmp_path, "quality"))
    s.start()
    try:
        assert s.score("quality.wav") == 0.5
    finally:
        s.close()


def test_scorer_error_line_raises(tmp_path):
    s = Scorer(spec_for(tmp_path))
    s.start()
    try:
        with pytest.raises(ScorerError, match="cannot read err.wav"):
            s.score("err.wav")
        assert s.score("ok.wav") == [1.0, 2.0, 3.0]  # still usable after a per-file error
    finally:
        s.close()


def test_scorer_timeout_and_death(tmp_path, monkeypatch):
    monkeypatch.setattr("lush_worker.scoring.STOP_GRACE", 0.2)
    dead = ScorerSpec("d", "D", "1", [sys.executable, "-c", "import sys; sys.exit(3)"], "embedding", tmp_path)
    s = Scorer(dead)
    s.start()
    with pytest.raises(ScorerError, match="exited"):
        s.score("a.wav")
    s.close()
    hang = ScorerSpec("h", "H", "1", [sys.executable, "-c", "import time; time.sleep(30)"], "embedding", tmp_path)
    s = Scorer(hang, timeout=0.5)
    s.start()
    with pytest.raises(ScorerError, match="no answer"):
        s.score("a.wav")
    s.close()


def test_load_scorers_skips_broken(tmp_path, caplog):
    ok = tmp_path / "a_ok"  # sorts before the duplicate-key dir
    ok.mkdir()
    (ok / "scorer.json").write_text(json.dumps(
        {"key": "mert", "name": "MERT", "command": ["python3", "run.py"], "provides": "embedding"}))
    for i, content in enumerate(["{", json.dumps({"key": "x"}),
                                 json.dumps({"key": "y", "name": "Y", "command": ["p"], "provides": "bogus"}),
                                 json.dumps({"key": "mert", "name": "dup", "command": ["p"], "provides": "quality"})]):
        d = tmp_path / f"bad{i}"
        d.mkdir()
        (d / "scorer.json").write_text(content)
    with caplog.at_level("WARNING"):
        specs = load_scorers(tmp_path)
    assert list(specs) == ["mert"] and specs["mert"].provides == "embedding"
    assert specs["mert"].directory == ok
    assert load_scorers(tmp_path / "nope") == {} and load_scorers(None) == {}


def test_register_reports_installed_scorers(runner, tmp_path):
    assert not [m for m in runner.registration(True)["models"] if m["engine"] == "scorer"]
    runner.scorers = load_scorers(_write_scorer(tmp_path))
    reg = runner.registration(True)
    assert {"engine": "scorer", "key": "mert", "name": "MERT", "version": "1",
            "capabilities": ["embedding"]} in reg["models"]
    assert reg["job_types"] == ["generate"]  # eval_lora arrives with a later task


def _write_scorer(tmp_path: Path) -> Path:
    d = tmp_path / "mert"
    d.mkdir()
    (d / "scorer.json").write_text(json.dumps(
        {"key": "mert", "name": "MERT", "command": ["python3", "run.py"], "provides": "embedding"}))
    return tmp_path
