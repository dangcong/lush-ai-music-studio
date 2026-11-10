from __future__ import annotations

import wave

import pytest

from lush_worker.audio import probe_wav
from lush_worker.engines.base import EngineError, JobCancelled
from lush_worker.engines.mock import STEPS, MockEngine

from .conftest import FakeCtx, generate_payload


def run(tmp_path, name, **kw):
    workdir = tmp_path / name
    workdir.mkdir()
    ctx = FakeCtx(workdir)
    return MockEngine(step_delay=0.0).generate(ctx, generate_payload(**kw)), ctx


def test_same_seed_same_file_and_different_seed_differs(tmp_path):
    a, _ = run(tmp_path, "a", seed=42)
    b, _ = run(tmp_path, "b", seed=42)
    c, _ = run(tmp_path, "c", seed=43)
    assert a.audio_path.read_bytes() == b.audio_path.read_bytes()
    assert a.audio_path.read_bytes() != c.audio_path.read_bytes()
    assert a.seed == 42


def test_wav_is_valid_and_respects_duration(tmp_path):
    res, ctx = run(tmp_path, "a", duration=3)
    with wave.open(str(res.audio_path)) as w:
        assert (w.getnchannels(), w.getsampwidth()) == (1, 2)
        assert w.getnframes() / w.getframerate() == pytest.approx(3.0, abs=0.001)
        assert w.getframerate() == res.sample_rate
        frames = w.readframes(w.getnframes())
    assert max(frames) > 0  # audible, not silence
    assert res.duration == pytest.approx(3.0, abs=0.001)
    assert probe_wav(res.audio_path) == (res.sample_rate, pytest.approx(3.0, abs=0.001))
    values = [v for v, _, _ in ctx.events]
    assert values == sorted(values) and sum("diffusion" in m for m in ctx.messages) == STEPS


def test_bpm_and_key_change_the_audio(tmp_path):
    base, _ = run(tmp_path, "a", bpm=80, key="A minor")
    fast, _ = run(tmp_path, "b", bpm=140, key="A minor")
    major, _ = run(tmp_path, "c", bpm=80, key="C major")
    assert base.audio_path.read_bytes() != fast.audio_path.read_bytes()
    assert base.audio_path.read_bytes() != major.audio_path.read_bytes()
    assert base.engine_info["bpm"] == 80 and fast.engine_info["bpm"] == 140


def test_null_bpm_and_key_are_derived_from_seed(tmp_path):
    a, _ = run(tmp_path, "a", bpm=None, key=None, time_signature=None)
    b, _ = run(tmp_path, "b", bpm=None, key=None, time_signature=None)
    assert a.audio_path.read_bytes() == b.audio_path.read_bytes()
    assert 70 <= a.engine_info["bpm"] <= 120 and a.engine_info["key"]


def test_cancel_between_steps(tmp_path):
    ctx = FakeCtx(tmp_path)

    def cancel_on_third_step(c):
        if any("diffusion 3/" in m for m in c.messages):
            c.cancel_requested = True
    ctx.on_progress = cancel_on_third_step
    with pytest.raises(JobCancelled):
        MockEngine(step_delay=0.0).generate(ctx, generate_payload())
    assert not any("diffusion 4/" in m for m in ctx.messages)
    assert not (tmp_path / "master.wav").exists()


def test_step_delay_is_applied_per_step(tmp_path):
    slept = []
    MockEngine(step_delay=0.25, sleep=slept.append).generate(FakeCtx(tmp_path), generate_payload())
    assert slept == [0.25] * STEPS


@pytest.mark.parametrize("kw", [{"duration": 0}, {"duration": 601}, {"duration": "abc"}, {"bpm": 5},
                                {"seed": None}])
def test_invalid_parameters_are_not_retryable(tmp_path, kw):
    with pytest.raises(EngineError) as e:
        MockEngine(step_delay=0.0).generate(FakeCtx(tmp_path), generate_payload(**kw))
    assert e.value.retryable is False


def test_model_reported():
    assert MockEngine().list_models() == [{"engine": "mock", "key": "mock-v1", "name": "Mock Engine v1",
                                           "capabilities": ["generate"]}]
