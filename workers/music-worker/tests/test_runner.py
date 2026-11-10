"""Runner against a fake Control Plane (respx)."""
from __future__ import annotations

import hashlib
import io
import json
import wave

import httpx
import pytest
import respx

import lush_worker.runner as runner_mod
from lush_worker.engines.base import Engine, EngineError, EngineUnavailable, GenerateResult
from lush_worker.runner import ProtocolMismatch

from .conftest import BASE, JOB_ID, LEASE, REGISTER_OK, TOKEN, make_job, parse_multipart

JOB = f"{BASE}/jobs/{JOB_ID}"


def err(status: int, code: str) -> httpx.Response:
    return httpx.Response(status, json={"error": {"code": code, "message": code}})


def body(call) -> dict:
    return json.loads(call.request.content)


@pytest.fixture(autouse=True)
def no_ffmpeg(monkeypatch):
    """Default: behave as if ffmpeg is missing (individual tests override)."""
    monkeypatch.setattr(runner_mod, "make_mp3_preview", lambda wav, dest: None)


@pytest.fixture
def cp():
    with respx.mock(assert_all_called=False) as router:
        router.post(f"{JOB}/progress").respond(json={"cancel_requested": False})
        router.post(f"{JOB}/artifacts").respond(json={"artifact_id": "a-1"})
        router.post(f"{JOB}/complete").respond(json={})
        router.post(f"{JOB}/fail").respond(json={})
        router.post(f"{JOB}/release").respond(json={})
        yield router


def route(cp, suffix: str):
    return cp.post(f"{JOB}/{suffix}")


class StubEngine(Engine):
    name = "mock"
    version = "t"
    capabilities = ("generate",)

    def __init__(self, action):
        self.action = action

    def list_models(self):
        return []

    def generate(self, ctx, payload):
        return self.action(ctx, payload)


# ---------------- happy path ----------------
def test_claim_progress_upload_complete(runner, cp):
    cp.post(f"{BASE}/jobs/claim").respond(json=make_job(duration=2))
    assert runner.run_once() is True

    claim = cp.post(f"{BASE}/jobs/claim").calls[0].request
    assert claim.headers["Authorization"] == f"Bearer {TOKEN}" and json.loads(claim.content) == {}

    progress = [body(c) for c in route(cp, "progress").calls]
    assert all(p["lease_token"] == LEASE and 0.0 <= p["progress"] <= 1.0 for p in progress)
    assert progress[0]["stage"] == "preparing" and progress[1]["stage"] == "running"
    values = [p["progress"] for p in progress]
    assert values == sorted(values) and len(values) >= 8
    assert any("diffusion 3/8" in p.get("message", "") for p in progress)

    uploads = route(cp, "artifacts").calls
    assert len(uploads) == 1  # no ffmpeg -> only the master
    form = parse_multipart(uploads[0].request)
    assert form["lease_token"] == LEASE and form["kind"] == "audio_master"
    assert form["sha256"] == hashlib.sha256(form["file"]).hexdigest()
    assert form["_filename"].endswith(".wav")
    with wave.open(io.BytesIO(form["file"])) as w:
        assert w.getnframes() / w.getframerate() == pytest.approx(2.0, abs=0.001)
        sample_rate = w.getframerate()
        assert any(w.readframes(w.getnframes()))  # not silence

    done = body(route(cp, "complete").calls[0])
    assert done["lease_token"] == LEASE
    result = done["result"]
    assert result["seed"] == 123456 and result["duration"] == pytest.approx(2.0, abs=0.001)
    assert result["sample_rate"] == sample_rate
    assert result["engine"] == "mock" and result["model_key"] == "mock-v1"
    assert result["engine_info"]["bpm"] == 80
    assert not route(cp, "fail").called
    assert not (runner.config.work_dir / "jobs" / JOB_ID).exists()  # scratch dir cleaned up


def test_claim_204_means_no_job(runner, cp):
    cp.post(f"{BASE}/jobs/claim").respond(204)
    assert runner.run_once() is False


def test_preview_uploaded_when_ffmpeg_produces_one(runner, cp, monkeypatch):
    def fake_preview(wav, dest):
        dest.write_bytes(b"ID3-test-preview")
        return dest
    monkeypatch.setattr(runner_mod, "make_mp3_preview", fake_preview)
    runner.run_job(make_job())
    forms = [parse_multipart(c.request) for c in route(cp, "artifacts").calls]
    assert [f["kind"] for f in forms] == ["audio_master", "audio_preview"]
    assert forms[1]["file"] == b"ID3-test-preview"
    assert forms[1]["sha256"] == hashlib.sha256(b"ID3-test-preview").hexdigest()
    assert route(cp, "complete").called


def test_preview_not_attempted_when_mp3_not_requested(runner, cp, monkeypatch):
    monkeypatch.setattr(runner_mod, "make_mp3_preview",
                        lambda wav, dest: pytest.fail("preview must not be built"))
    runner.run_job(make_job(output={"formats": ["wav"]}))
    assert len(route(cp, "artifacts").calls) == 1 and route(cp, "complete").called


def test_progress_failures_never_break_the_job(runner, cp):
    route(cp, "progress").mock(side_effect=httpx.ConnectError("down"))
    runner.run_job(make_job())
    assert route(cp, "complete").called and not route(cp, "fail").called


# ---------------- lease lost / cancel / shutdown ----------------
def test_lease_lost_drops_job_without_upload(runner, cp):
    route(cp, "progress").mock(side_effect=[
        httpx.Response(200, json={"cancel_requested": False}),
        httpx.Response(200, json={"cancel_requested": False}),
        err(409, "lease_lost"),
    ])
    runner.run_job(make_job())
    assert len(route(cp, "progress").calls) == 3  # no further reports once the lease is gone
    for suffix in ("artifacts", "complete", "fail", "release"):
        assert not route(cp, suffix).called, suffix


def test_lease_lost_on_upload_drops_job(runner, cp):
    route(cp, "artifacts").mock(return_value=err(409, "lease_lost"))
    runner.run_job(make_job())
    assert len(route(cp, "artifacts").calls) == 1
    assert not route(cp, "complete").called and not route(cp, "fail").called


def test_cancel_requested_via_progress(runner, cp):
    route(cp, "progress").mock(side_effect=[
        httpx.Response(200, json={"cancel_requested": False}),
        httpx.Response(200, json={"cancel_requested": False}),
        httpx.Response(200, json={"cancel_requested": True}),
    ] + [httpx.Response(200, json={"cancel_requested": True})] * 20)
    runner.run_job(make_job())
    assert not route(cp, "artifacts").called and not route(cp, "complete").called
    fail = body(route(cp, "fail").calls[0])
    assert fail["cancelled"] is True and fail["retryable"] is False and fail["lease_token"] == LEASE
    assert len(route(cp, "progress").calls) < 8  # stopped mid-way, not after all steps


def test_cancel_requested_via_heartbeat(runner, cp):
    hb = cp.post(f"{BASE}/heartbeat").respond(json={"cancel_job_ids": [JOB_ID], "maintenance": False})

    def action(ctx, payload):
        runner.heartbeat_once()  # what the heartbeat thread does while a job runs
        ctx.raise_if_aborted()
        pytest.fail("job should have been cancelled")

    runner.engines["mock"] = StubEngine(action)
    runner.run_job(make_job())
    assert body(hb.calls[0]) == {"current_job_id": JOB_ID, "gpu": None}
    assert body(route(cp, "fail").calls[0])["cancelled"] is True
    assert not route(cp, "artifacts").called


def test_heartbeat_idle_sends_gpu_stats(runner, cp):
    hb = cp.post(f"{BASE}/heartbeat").respond(json={"cancel_job_ids": [], "maintenance": False})
    runner._gpu_query = lambda: {"name": "NVIDIA GeForce RTX 3060", "utilization": 37.0,
                                 "vram_used_mb": 512, "vram_total_mb": 12288}
    runner.heartbeat_once()
    assert body(hb.calls[0]) == {"current_job_id": None,
                                 "gpu": {"utilization": 37.0, "vram_used_mb": 512, "vram_total_mb": 12288}}


def test_shutdown_releases_job(runner, cp):
    def action(ctx, payload):
        runner.stop()  # SIGTERM / Ctrl+C arrives mid-job
        ctx.raise_if_aborted()

    runner.engines["mock"] = StubEngine(action)
    runner.run_job(make_job())
    assert body(route(cp, "release").calls[0]) == {"lease_token": LEASE}
    for suffix in ("artifacts", "complete", "fail"):
        assert not route(cp, suffix).called


def test_keyboard_interrupt_releases_job(runner, cp):
    def action(ctx, payload):
        raise KeyboardInterrupt

    runner.engines["mock"] = StubEngine(action)
    runner.run_job(make_job())
    assert route(cp, "release").called and runner.stopping


# ---------------- engine errors ----------------
@pytest.mark.parametrize("exc, retryable", [
    (EngineUnavailable("engine is starting"), True),
    (EngineError("bad parameter"), False),
    (RuntimeError("boom"), False),
])
def test_engine_error_reports_fail_with_retryable(runner, cp, exc, retryable):
    def action(ctx, payload):
        raise exc

    runner.engines["mock"] = StubEngine(action)
    runner.run_job(make_job())
    fail = body(route(cp, "fail").calls[0])
    assert fail["retryable"] is retryable and fail["cancelled"] is False
    assert str(exc) in fail["error"]
    assert not route(cp, "artifacts").called and not route(cp, "complete").called


def test_invalid_duration_is_not_retryable(runner, cp):
    runner.run_job(make_job(duration=0))
    fail = body(route(cp, "fail").calls[0])
    assert fail["retryable"] is False and "duration" in fail["error"]


def test_unknown_engine_fails_job(runner, cp):
    runner.run_job(make_job(model={"id": "m", "engine": "acestep", "key": "x"}))
    fail = body(route(cp, "fail").calls[0])
    assert fail["retryable"] is False and "acestep" in fail["error"]


# ---------------- upload / complete retries ----------------
def test_checksum_mismatch_retries_upload(runner, cp):
    route(cp, "artifacts").mock(side_effect=[err(422, "checksum_mismatch"), httpx.ConnectError("reset"),
                                             httpx.Response(200, json={"artifact_id": "a-1"})])
    runner.run_job(make_job())
    calls = route(cp, "artifacts").calls
    assert len(calls) == 3
    forms = [parse_multipart(c.request) for c in calls]
    assert forms[0]["file"] == forms[2]["file"] and forms[0]["sha256"] == forms[2]["sha256"]
    assert route(cp, "complete").called and not route(cp, "fail").called


def test_upload_gives_up_and_fails_retryable(runner, cp):
    route(cp, "artifacts").mock(return_value=err(422, "checksum_mismatch"))
    runner.run_job(make_job())
    assert len(route(cp, "artifacts").calls) == runner_mod.UPLOAD_ATTEMPTS
    assert not route(cp, "complete").called
    assert "checksum_mismatch" in body(route(cp, "fail").calls[0])["error"]


def test_complete_retries_on_network_error(runner, cp):
    route(cp, "complete").mock(side_effect=[httpx.ConnectError("reset"), httpx.Response(200, json={})])
    runner.run_job(make_job())
    assert len(route(cp, "complete").calls) == 2 and not route(cp, "fail").called


# ---------------- register ----------------
def test_registration_body(runner):
    runner._gpu_query = lambda: {"name": "NVIDIA GeForce RTX 3060", "utilization": 0.0,
                                 "vram_used_mb": 512, "vram_total_mb": 12288}
    assert runner.registration(fresh_start=True) == {
        "protocol_version": 1, "worker_version": "0.1.0", "fresh_start": True,
        "job_types": ["generate"],
        "engines": [{"name": "mock", "version": "1", "capabilities": ["generate"]}],
        "models": [{"engine": "mock", "key": "mock-v1", "name": "Mock Engine v1", "capabilities": ["generate"]}],
        "loras": [],
        "gpu": {"name": "NVIDIA GeForce RTX 3060", "vram_total_mb": 12288},
    }


def test_register_retries_until_success(runner, cp):
    reg = cp.post(f"{BASE}/register").mock(side_effect=[
        httpx.ConnectError("refused"), httpx.Response(502, text="bad gateway"), err(401, "unauthorized"),
        httpx.Response(200, json={**REGISTER_OK, "heartbeat_interval": 7, "poll_interval": 2}),
    ])
    assert runner.register_with_retry() is True
    assert len(reg.calls) == 4
    assert body(reg.calls[3])["fresh_start"] is True
    assert runner.heartbeat_interval == 7 and runner.poll_interval == 2


def test_register_protocol_mismatch_is_fatal(runner, cp):
    cp.post(f"{BASE}/register").mock(return_value=err(409, "protocol_mismatch"))
    with pytest.raises(ProtocolMismatch):
        runner.register_with_retry()


def test_engine_down_registers_without_its_models(runner, cp):
    def boom():
        raise EngineUnavailable("down")
    runner.engines["mock"].list_models = boom
    reg = cp.post(f"{BASE}/register").respond(json=REGISTER_OK)
    assert runner.register_with_retry() is True
    assert body(reg.calls[0])["models"] == [] and body(reg.calls[0])["engines"][0]["name"] == "mock"


# ---------------- main loop ----------------
def test_run_forever_survives_auth_error_and_outage(runner, cp, monkeypatch):
    monkeypatch.setattr(runner, "_heartbeat_loop", lambda: None)
    reg = cp.post(f"{BASE}/register").respond(json=REGISTER_OK)

    def last_claim(request):
        runner.stop()
        return httpx.Response(204)

    claim = cp.post(f"{BASE}/jobs/claim").mock(side_effect=[
        err(403, "node_disabled"), httpx.ConnectError("refused"), httpx.Response(200, json=make_job()),
        last_claim])
    runner.run_forever()
    assert len(claim.calls) == 4
    assert len(reg.calls) == 2  # initial + re-register after the outage
    assert route(cp, "complete").called
