"""caption_tracks: parsing, discovery, subprocess protocol (real fake_captioner.py) and runner wiring."""
from __future__ import annotations

import json
import sys
from pathlib import Path

import httpx
import pytest
import respx

import lush_worker.runner as runner_mod
from lush_worker.api_client import ControlPlaneClient
from lush_worker.captioning import (CaptionerSpec, discover_captioners, extract_fields, parse_output,
                                    run_caption_job)

from .conftest import BASE, CP, JOB_ID, LEASE, TOKEN, FakeCtx

FAKE = Path(__file__).parent / "fake_captioner.py"
JOB = f"{BASE}/jobs/{JOB_ID}"


def spec_for(tmp_path, mode="json") -> CaptionerSpec:
    return CaptionerSpec("fake-cap", "Fake", "1", sys.executable, FAKE, mode, tmp_path)


def payload_for(tmp_path, *names: str) -> dict:
    tracks, inputs = [], {}
    for i, n in enumerate(names):
        (tmp_path / n).write_bytes(b"x")
        tracks.append({"catalog_track_id": f"t{i}", "input": f"r{i}"})
        inputs[f"r{i}"] = {"path": str(tmp_path / n)}
    return {"tracks": tracks, "inputs": inputs}


@pytest.fixture
def fake_env(tmp_path, monkeypatch):
    monkeypatch.setenv("FAKE_CAPTIONER_STATE", str(tmp_path))
    monkeypatch.setenv("FAKE_CAPTIONER_LOG", str(tmp_path / "requests.log"))
    return tmp_path / "requests.log"


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


@pytest.fixture
def api():
    return ControlPlaneClient(CP, TOKEN)


# ---------------- parsing ----------------
def test_parse_output_json_and_text_modes():
    j = 'Sure! {"mood": "calm", "instruments": ["piano", "bass"], "vocals": "Instrumental", "caption": "Soft."} bye'
    assert parse_output(j, "json") == {"mood": "calm", "instruments": "piano, bass",
                                       "vocals": "Instrumental", "caption": "Soft."}
    assert parse_output("{broken", "json") is None
    assert parse_output("", "json") is None
    assert parse_output('{"mood": "x"}', "json") is None
    t = parse_output("  A mellow piano tune with vocals.  ", "text")
    assert t == {"mood": "mellow", "instruments": "piano", "vocals": "Vocals",
                 "caption": "A mellow piano tune with vocals."}
    assert parse_output("   ", "text") is None


def test_extract_fields_from_caption():
    f = extract_fields("A mellow lo-fi instrumental with warm Rhodes, upright bass and brushed drums, nostalgic")
    assert f["instruments"] == "Rhodes, upright bass, brushed drums"
    assert f["mood"] == "mellow, warm, nostalgic"
    assert f["vocals"] == "Instrumental"
    assert extract_fields("Dreamy pad, no words")["vocals"] == ""
    assert extract_fields("instrumental with a sung hook")["vocals"] == "Vocals"
    assert "bass" not in extract_fields("a bassline")["instruments"]


# ---------------- discovery / registration ----------------
def test_discover_captioners_skips_broken_config(tmp_path, caplog):
    ok = tmp_path / "ok"
    ok.mkdir()
    (ok / "run.py").write_text("")
    (ok / "captioner.json").write_text(json.dumps(
        {"key": "k1", "name": "K1", "version": "2", "python": "python3", "script": "run.py", "mode": "text"}))
    for name, content in {"broken": "{nope", "nomode": json.dumps({"key": "k2", "name": "n", "version": "1",
                          "python": "p", "script": "run.py"}),
                          "noscript": json.dumps({"key": "k3", "name": "n", "version": "1", "python": "p",
                                                  "script": "missing.py", "mode": "json"})}.items():
        d = tmp_path / name
        d.mkdir()
        (d / "captioner.json").write_text(content)
    with caplog.at_level("WARNING"):
        specs = discover_captioners(tmp_path)
    assert [s.key for s in specs] == ["k1"]
    assert specs[0].script == ok / "run.py" and specs[0].python == "python3" and specs[0].directory == ok
    assert sum("ignored" in r.message for r in caplog.records) == 3
    assert discover_captioners(tmp_path / "nope") == [] and discover_captioners(None) == []


def test_registration_reports_captioner_models(runner, tmp_path):
    assert "caption_tracks" not in runner.registration(True)["job_types"]
    runner.captioners = [spec_for(tmp_path)]
    reg = runner.registration(True)
    assert reg["job_types"] == ["generate", "caption_tracks"]
    assert {"engine": "captioner", "key": "fake-cap", "name": "Fake", "version": "1",
            "capabilities": ["caption"]} in reg["models"]


# ---------------- run_caption_job ----------------
def test_run_caption_job_posts_each_track(tmp_path, api, cp, fake_env):
    ctx = FakeCtx(tmp_path)
    res = run_caption_job(ctx, api, spec_for(tmp_path), payload_for(tmp_path, "good1.wav", "good2.wav"),
                          track_timeout=10)
    assert res == {"done": 2, "failed": []}
    p = posts(cp)
    assert [x["catalog_track_id"] for x in p] == ["t0", "t1"]
    assert p[0]["caption"] == "A calm piano piece." and p[0]["source_ref"] == "fake-cap@1"
    assert p[0]["lease_token"] == LEASE and p[0]["vocals"] == "Instrumental"
    assert [r[3] for r in ctx.reports] == ["caption", "caption"] and ctx.reports[-1][0] == 1.0


def test_bad_json_retried_then_raw_text(tmp_path, api, cp, fake_env):
    res = run_caption_job(FakeCtx(tmp_path), api, spec_for(tmp_path), payload_for(tmp_path, "bad.wav"),
                          track_timeout=10)
    assert res == {"done": 1, "failed": []}
    assert fake_env.read_text().split() == ["bad.wav", "bad.wav"]
    assert posts(cp)[0]["caption"] == "not json"


def test_empty_output_reports_error_and_continues(tmp_path, api, cp, fake_env):
    res = run_caption_job(FakeCtx(tmp_path), api, spec_for(tmp_path),
                          payload_for(tmp_path, "empty.wav", "good.wav"), track_timeout=10)
    assert res == {"done": 1, "failed": ["t0"]}
    p = posts(cp)
    assert "error" in p[0] and "caption" not in p[0] and p[1]["caption"] == "A calm piano piece."


def test_crash_restarts_once(tmp_path, api, cp, fake_env):
    res = run_caption_job(FakeCtx(tmp_path), api, spec_for(tmp_path),
                          payload_for(tmp_path, "crash.wav", "good.wav"), track_timeout=10)
    assert res == {"done": 1, "failed": ["t0"]}
    p = posts(cp)
    assert "exited" in p[0]["error"] and p[1]["catalog_track_id"] == "t1" and "caption" in p[1]


def test_text_mode_uses_text_as_caption(tmp_path, api, cp, fake_env):
    res = run_caption_job(FakeCtx(tmp_path), api, spec_for(tmp_path, "text"),
                          payload_for(tmp_path, "bad.wav"), track_timeout=10)
    assert res["done"] == 1 and posts(cp)[0]["caption"] == "not json"


def test_cancel_stops_subprocess_between_tracks(tmp_path, api, cp, fake_env):
    from lush_worker.engines.base import JobCancelled
    ctx = FakeCtx(tmp_path)
    orig = ctx.report

    def report(*a, **k):
        orig(*a, **k)
        ctx.cancel_requested = True
    ctx.report = report
    with pytest.raises(JobCancelled):
        run_caption_job(ctx, api, spec_for(tmp_path), payload_for(tmp_path, "good1.wav", "good2.wav"),
                        track_timeout=10)
    assert len(posts(cp)) == 1


def test_timeout_is_a_track_error(tmp_path, api, cp, fake_env):
    hang = tmp_path / "hang.py"
    hang.write_text("import sys,time\nfor l in sys.stdin:\n    time.sleep(60)\n")
    spec = CaptionerSpec("h", "H", "1", sys.executable, hang, "json", tmp_path)
    res = run_caption_job(FakeCtx(tmp_path), api, spec, payload_for(tmp_path, "good.wav"), track_timeout=1)
    assert res == {"done": 0, "failed": ["t0"]} and "no answer" in posts(cp)[0]["error"]


# ---------------- runner ----------------
def caption_job(sha: str) -> dict:
    return {"id": JOB_ID, "type": "caption_tracks", "attempt": 1, "lease_token": LEASE,
            "lease_expires_at": "2026-01-01T00:00:00.000Z",
            "payload": {"catalog_id": "c", "model": {"id": "m", "key": "fake-cap"},
                        "tracks": [{"catalog_track_id": "t0", "input": "t0"}],
                        "inputs": {"t0": {"artifact_id": "art-1", "filename": "good.wav", "size_bytes": 1,
                                          "sha256": sha}}}}


def test_runner_caption_job_end_to_end(runner, cp, tmp_path, fake_env):
    import hashlib
    runner.captioners = [spec_for(tmp_path)]
    runner.caption_timeout = 10
    cp.get(f"{JOB}/inputs/art-1").respond(content=b"x")
    runner.run_job(caption_job(hashlib.sha256(b"x").hexdigest()))
    assert len(posts(cp)) == 1
    done = json.loads(cp.post(f"{JOB}/complete").calls[0].request.content)
    assert done["result"] == {"done": 1, "failed": []}


def test_runner_fails_when_captioner_missing(runner, cp):
    runner.run_job(caption_job("0" * 64))
    fail = json.loads(cp.post(f"{JOB}/fail").calls[0].request.content)
    assert fail["retryable"] is False and fail["error"] == "Captioner fake-cap không có trên máy này"
