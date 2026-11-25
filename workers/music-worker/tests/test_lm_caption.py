"""acestep-lm: catalog captions and Reference Audio prompts written by ACE-Step's LM (`full_analysis_only`).
The analysis result shape follows acestep/api/job_analysis_runtime.py at ca1e85f (mock, not a real run)."""
from __future__ import annotations

import hashlib
import json

import pytest
import respx

import lush_worker.runner as runner_mod
from lush_worker.api_client import ControlPlaneClient
from lush_worker.engines.acestep import AceStepClient, AceStepEngine, AceStepError
from lush_worker.lm_caption import (LM_CAPTIONER_KEY, lm_captioner_model, run_lm_caption_job,
                                    run_lm_describe_job)

from .conftest import BASE, CP, JOB_ID, LEASE, TOKEN, FakeCtx

JOB = f"{BASE}/jobs/{JOB_ID}"
ACE = "http://ace.test:8001"
LM = "acestep-5Hz-lm-4B"
LM_RESULT = {
    "status_message": "Full Hardware Analysis Success", "bpm": 82, "keyscale": "A minor",
    "timesignature": "4", "duration": 150, "genre": "lo-fi",
    "prompt": "A mellow lo-fi hip hop beat with warm Rhodes piano, vinyl crackle and brushed drums.",
    "lyrics": "[Instrumental]", "language": "unknown", "audio_paths": [],
}


def env(data):
    return {"code": 200, "error": None, "data": data}


@pytest.fixture
def cp():
    with respx.mock(assert_all_called=False) as router:
        router.post(f"{JOB}/progress").respond(json={"cancel_requested": False})
        router.post(f"{JOB}/complete").respond(json={})
        router.post(f"{JOB}/fail").respond(json={})
        router.post(f"{JOB}/captions").respond(json={"revision_seq": 1})
        yield router


def posts(cp) -> list[dict]:
    return [json.loads(c.request.content) for c in cp.post(f"{JOB}/captions").calls]


def tracks(tmp_path, *names):
    out, inputs = [], {}
    for i, n in enumerate(names):
        (tmp_path / n).write_bytes(b"x")
        out.append({"catalog_track_id": f"t{i}", "input": f"r{i}"})
        inputs[f"r{i}"] = {"path": str(tmp_path / n)}
    return {"model": {"key": LM_CAPTIONER_KEY}, "tracks": out, "inputs": inputs}


def describe_payload(tmp_path):
    (tmp_path / "ref.wav").write_bytes(b"x")
    return {"model": {"key": LM_CAPTIONER_KEY}, "inputs": {"audio": {"path": str(tmp_path / "ref.wav")}}}


def test_lm_captioner_model():
    assert lm_captioner_model(LM) == {"engine": "captioner", "key": "acestep-lm", "name": "ACE-Step LM 4B",
                                      "version": LM, "capabilities": ["caption"]}


def test_describe_maps_prompt_bpm_key(tmp_path):
    res = run_lm_describe_job(FakeCtx(tmp_path), lambda ctx, path: dict(LM_RESULT), LM, describe_payload(tmp_path))
    assert res["caption"] == LM_RESULT["prompt"]
    assert res["tempo_bpm"] == 82 and res["key"] == "A minor"
    assert res["source_ref"] == "acestep-lm@acestep-5Hz-lm-4B"
    assert res["mood"] == "mellow, warm" and "piano" in res["instruments"]


def test_describe_empty_prompt_is_an_engine_error(tmp_path):
    from lush_worker.engines.base import EngineError
    with pytest.raises(EngineError, match="empty"):
        run_lm_describe_job(FakeCtx(tmp_path), lambda ctx, path: {**LM_RESULT, "prompt": " "}, LM,
                            describe_payload(tmp_path))


def test_caption_job_empty_prompt_counts_failed(tmp_path, cp):
    answers = iter([dict(LM_RESULT), {**LM_RESULT, "prompt": ""}])
    res = run_lm_caption_job(FakeCtx(tmp_path), ControlPlaneClient(CP, TOKEN), lambda ctx, path: next(answers),
                             LM, tracks(tmp_path, "a.wav", "b.wav"))
    assert res == {"done": 1, "failed": ["t1"]}
    p = posts(cp)
    assert p[0]["caption"] == LM_RESULT["prompt"] and p[0]["source_ref"] == "acestep-lm@acestep-5Hz-lm-4B"
    assert "error" in p[1] and "caption" not in p[1]


def test_caption_job_analysis_error_counts_failed(tmp_path, cp):
    def analyze(ctx, path):
        if path.name == "a.wav":
            raise AceStepError("ACE-Step task t-1 failed: LLM Understanding failed")
        return dict(LM_RESULT)
    res = run_lm_caption_job(FakeCtx(tmp_path), ControlPlaneClient(CP, TOKEN), analyze, LM,
                             tracks(tmp_path, "a.wav", "b.wav"))
    assert res == {"done": 1, "failed": ["t0"]}
    assert "LLM Understanding failed" in posts(cp)[0]["error"]


# ---------------- AceStepEngine.analyze_audio ----------------
@respx.mock
def test_analyze_audio_posts_full_analysis_and_returns_metadata(tmp_path):
    audio = tmp_path / "song.mp3"
    audio.write_bytes(b"mp3")
    respx.get(f"{ACE}/health").respond(json=env({"status": "ok"}))
    respx.get(f"{ACE}/v1/model_inventory").respond(json=env({"models": [], "llm_initialized": True,
                                                              "loaded_lm_model": LM}))
    submit = respx.post(f"{ACE}/release_task").respond(json=env({"task_id": "t-1"}))
    respx.post(f"{ACE}/query_result").respond(json=env([
        {"task_id": "t-1", "status": 1, "result": json.dumps([LM_RESULT])}]))
    engine = AceStepEngine(AceStepClient(ACE), poll_interval=0, sleep=lambda s: None)
    item = engine.analyze_audio(FakeCtx(tmp_path), audio)
    assert item["prompt"] == LM_RESULT["prompt"]
    sent = submit.calls[0].request.content
    assert b'name="full_analysis_only"' in sent and b"true" in sent and b'name="src_audio"' in sent


@respx.mock
def test_analyze_audio_without_lm_fails_before_submitting(tmp_path):
    audio = tmp_path / "song.mp3"
    audio.write_bytes(b"mp3")
    respx.get(f"{ACE}/health").respond(json=env({"status": "ok"}))
    respx.get(f"{ACE}/v1/model_inventory").respond(json=env({"models": [], "llm_initialized": False}))
    submit = respx.post(f"{ACE}/release_task").respond(json=env({"task_id": "t-1"}))
    engine = AceStepEngine(AceStepClient(ACE), poll_interval=0, sleep=lambda s: None)
    with pytest.raises(AceStepError, match="LM chưa nạp"):
        engine.analyze_audio(FakeCtx(tmp_path), audio)
    assert not submit.called


# ---------------- runner wiring ----------------
class LmEngine(AceStepEngine):
    name = "acestep"

    def __init__(self, lm, down=None):
        self.lm = lm
        self.down = down
        self.analyzed = []

    def health(self):
        return self.down

    def list_models(self):
        return []

    def list_loras(self):
        return []

    def lm_status(self):
        return self.lm

    def analyze_audio(self, ctx, path):
        self.analyzed.append(path.name)
        return dict(LM_RESULT)


def test_registration_lists_acestep_lm_when_lm_loaded(runner):
    runner.engines["acestep"] = LmEngine(LM)
    reg = runner.registration(True)
    assert lm_captioner_model(LM) in reg["models"]
    assert "caption_tracks" in reg["job_types"] and "describe_input" in reg["job_types"]


def test_registration_without_lm_has_no_acestep_lm(runner):
    runner.engines["acestep"] = LmEngine(None)
    reg = runner.registration(True)
    assert all(m["key"] != "acestep-lm" for m in reg["models"])
    assert "caption_tracks" not in reg["job_types"]


def describe_job():
    return {"id": JOB_ID, "type": "describe_input", "attempt": 1, "lease_token": LEASE,
            "lease_expires_at": "2026-01-01T00:00:00.000Z",
            "payload": {"model": {"key": LM_CAPTIONER_KEY},
                        "inputs": {"audio": {"artifact_id": "art-1", "filename": "ref.wav", "size_bytes": 1,
                                             "sha256": hashlib.sha256(b"x").hexdigest()}}}}


def test_runner_describe_with_lm(runner, cp, monkeypatch):
    monkeypatch.setattr(runner_mod, "analysis_available", lambda: False)
    runner.engines["acestep"] = engine = LmEngine(LM)
    cp.get(f"{JOB}/inputs/art-1").respond(content=b"x")
    runner.run_job(describe_job())
    done = json.loads(cp.post(f"{JOB}/complete").calls[0].request.content)
    assert done["result"]["caption"] == LM_RESULT["prompt"] and len(engine.analyzed) == 1


def test_describe_lm_missing_fails(runner, cp):
    runner.engines["acestep"] = LmEngine(None)
    cp.get(f"{JOB}/inputs/art-1").respond(content=b"x")
    runner.run_job(describe_job())
    fail = json.loads(cp.post(f"{JOB}/fail").calls[0].request.content)
    assert fail["retryable"] is False and fail["error"] == "LM chưa nạp trên máy render"


def test_describe_when_acestep_down_is_retryable(runner, cp):
    runner.engines["acestep"] = LmEngine(None, down="cannot reach ACE-Step")
    cp.get(f"{JOB}/inputs/art-1").respond(content=b"x")
    runner.run_job(describe_job())
    fail = json.loads(cp.post(f"{JOB}/fail").calls[0].request.content)
    assert fail["retryable"] is True and "LM chưa nạp" not in fail["error"]
