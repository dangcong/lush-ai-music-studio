"""AceStepEngine against a fake ACE-Step HTTP server (respx). No real ACE-Step / GPU involved.
Response shapes follow the research repo's tests (taken from the ACE-Step 1.5 source)."""
from __future__ import annotations

import io
import json
import wave

import httpx
import pytest
import respx

from lush_worker.engines.acestep import (AceStepClient, AceStepEngine, AceStepError, AceStepUnavailable,
                                         adapter_name, build_release_body, build_release_files,
                                         parse_task_result)
from lush_worker.engines.base import JobCancelled, LeaseLost, ShutdownRequested

from .conftest import FakeCtx, generate_payload, parse_multipart

URL = "http://ace.test:8001"
TURBO = "acestep-v15-turbo"


def env(data, code=200, error=None):
    return {"data": data, "code": code, "error": error, "timestamp": 1700000000000, "extra": None}


HEALTH_OK = env({"status": "ok", "service": "ACE-Step API", "models_initialized": True, "loaded_model": TURBO})
INVENTORY = env({
    "models": [{"name": TURBO, "is_default": True, "is_loaded": True},
               {"name": "acestep-v15-xl-sft", "is_default": False, "is_loaded": False}],
    "default_model": TURBO,
    "lm_models": [{"name": "acestep-5Hz-lm-1.7B", "is_loaded": False}],
    "loaded_lm_model": None, "llm_initialized": False,
})


def wav_bytes(seconds: float = 1.0, rate: int = 48000) -> bytes:
    buf = io.BytesIO()
    with wave.open(buf, "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(rate)
        w.writeframes(b"\x01\x00" * 2 * int(rate * seconds))
    return buf.getvalue()


def payload(**kw):
    return generate_payload(**{"model": {"id": "m-9", "engine": "acestep", "key": TURBO}, "duration": 30,
                               "seed": 1234, "bpm": 72, "key": "C major", **kw})


def running(progress, text, task_id="t-1"):
    return httpx.Response(200, json=env([{"task_id": task_id, "status": 0, "progress_text": text,
                                          "result": json.dumps([{"file": "", "progress": progress,
                                                                 "stage": "running"}])}]))


def done(task_id="t-1", **item):
    item = {"file": "/v1/audio?path=%2Ftmp%2Ft-1_0.wav", "seed_value": "1234", "dit_model": TURBO, **item}
    return httpx.Response(200, json=env([{"task_id": task_id, "status": 1, "result": json.dumps([item])}]))


def lora_status(loaded=False, adapters=(), use=None):
    return env({"lora_loaded": loaded, "use_lora": loaded if use is None else use, "lora_scale": 1.0,
                "adapters": list(adapters)})


def lora_routes():
    """Record the order of LoRA calls. /v1/lora/unload must NEVER be called (broken in ACE-Step)."""
    calls = []

    def rec(name):
        def handler(request):
            calls.append((name, json.loads(request.content) if request.content else None))
            return httpx.Response(200, json=env({"message": "ok"}))
        return handler
    for name in ("load", "scale", "toggle", "unload"):
        respx.post(f"{URL}/v1/lora/{name}").mock(side_effect=rec(name))
    return calls


def mock_generation_ok():
    respx.get(f"{URL}/health").respond(json=HEALTH_OK)
    respx.post(f"{URL}/release_task").respond(json=env({"task_id": "t-1"}))
    respx.post(f"{URL}/query_result").mock(return_value=done())
    respx.get(f"{URL}/v1/audio").respond(content=wav_bytes())


@pytest.fixture
def lora_dir(tmp_path):
    d = tmp_path / "loras"
    (d / "chill-turbo").mkdir(parents=True)
    (d / "chill-turbo" / "adapter_config.json").write_text("{}")
    (d / "chill-turbo" / "adapter_model.safetensors").write_bytes(b"weights")
    (d / "not-a-lora").mkdir()
    return d


@pytest.fixture
def ace(lora_dir) -> AceStepEngine:
    return AceStepEngine(AceStepClient(URL, "secret"), timeout=600, lora_dir=lora_dir, poll_interval=0,
                         sleep=lambda s: None)


@pytest.fixture
def ctx(tmp_path) -> FakeCtx:
    workdir = tmp_path / "job"
    workdir.mkdir()
    return FakeCtx(workdir)


# ---------------- request body ----------------
def test_build_release_body_from_v1_payload():
    assert build_release_body(payload()) == {
        "prompt": "ambient piano, instrumental", "lyrics": "[Instrumental]", "vocal_language": "unknown",
        "task_type": "text2music", "audio_duration": 30.0, "model": TURBO,
        "thinking": False, "use_cot_caption": False, "use_cot_language": False,
        "use_random_seed": False, "seed": 1234, "batch_size": 1, "inference_steps": 8,
        "audio_format": "wav", "guidance_scale": 7, "lm_temperature": 0.85,
        "lm_negative_prompt": "NO USER INPUT", "bpm": 72, "key_scale": "C major", "time_signature": "4",
    }


def test_build_release_body_optional_fields_and_params():
    body = build_release_body(payload(bpm=None, key=None, time_signature=None, duration=5,
                                      model={"id": "m", "engine": "acestep", "key": "acestep-v15-base"},
                                      params={"inference_steps": 50, "guidance_scale": 3.5}))
    assert not {"bpm", "key_scale", "time_signature"} & set(body)
    assert body["audio_duration"] == 10.0  # ACE-Step minimum
    assert body["inference_steps"] == 50 and body["guidance_scale"] == 3.5
    assert build_release_body(payload(model={"id": "m", "engine": "acestep", "key": "acestep-v15-base"})
                              )["inference_steps"] == 32  # non-turbo default
    assert build_release_body(payload(time_signature="6/8"))["time_signature"] == "6"


@pytest.mark.parametrize("kw", [{"seed": -1}, {"seed": None}, {"duration": 601}, {"duration": "x"},
                                {"model": {}}, {"params": {"inference_steps": "many"}}])
def test_build_release_body_rejects_bad_params_as_not_retryable(kw):
    with pytest.raises(AceStepError) as e:
        build_release_body(payload(**kw))
    assert e.value.retryable is False


def test_parse_task_result():
    assert parse_task_result("") == []
    assert parse_task_result('[{"file": "a"}]') == [{"file": "a"}]
    assert parse_task_result('{"file": "a"}') == [{"file": "a"}]
    assert parse_task_result("not json") == []


# ---------------- models / health ----------------
@respx.mock
def test_list_models_only_loaded(ace):
    route = respx.get(f"{URL}/v1/model_inventory").respond(json=INVENTORY)
    assert ace.list_models() == [{"engine": "acestep", "key": TURBO, "name": TURBO,
                                  "capabilities": ["generate", "lora", "reference_audio", "cover"]}]
    assert route.calls[0].request.headers["Authorization"] == "Bearer secret"


@respx.mock
def test_list_models_lazy_server_uses_default(ace):
    data = json.loads(json.dumps(INVENTORY))
    for m in data["data"]["models"]:
        m["is_loaded"] = False
    respx.get(f"{URL}/v1/model_inventory").respond(json=data)
    assert [m["key"] for m in ace.list_models()] == [TURBO]


@respx.mock
def test_list_models_raises_when_down_and_health_reports_problem(ace):
    respx.get(f"{URL}/v1/model_inventory").mock(side_effect=httpx.ConnectError("refused"))
    respx.get(f"{URL}/health").mock(side_effect=httpx.ConnectError("refused"))
    with pytest.raises(AceStepUnavailable):
        ace.list_models()
    assert "not reachable" in ace.health()


@respx.mock
def test_error_envelope_and_detail(ace):
    respx.get(f"{URL}/v1/model_inventory").respond(json=env(None, code=500, error="model not loaded"))
    with pytest.raises(AceStepError, match="model not loaded"):
        ace.list_models()
    respx.post(f"{URL}/release_task").respond(status_code=401, json={"detail": "Invalid API key"})
    with pytest.raises(AceStepError, match="Invalid API key"):
        ace.client.release_task({"prompt": "x"})


def test_list_loras_only_adapter_dirs(ace):
    assert ace.list_loras() == ["chill-turbo"]
    assert AceStepEngine(AceStepClient(URL), lora_dir=None).list_loras() == []


# ---------------- generate: done / failed / timeout / unavailable ----------------
@respx.mock
def test_generate_full_flow(ace, ctx):
    respx.get(f"{URL}/health").respond(json=HEALTH_OK)
    respx.get(f"{URL}/v1/lora/status").respond(json=lora_status())
    release = respx.post(f"{URL}/release_task").respond(
        json=env({"task_id": "t-1", "status": "queued", "queue_position": 1}))
    query = respx.post(f"{URL}/query_result").mock(side_effect=[
        running(0.3, "Diffusion step 3/8"), running(0.7, "Decoding VAE"),
        done(metas={"bpm": 72, "duration": 30})])
    audio = respx.get(f"{URL}/v1/audio").respond(content=wav_bytes(1.5, 48000))

    res = ace.generate(ctx, payload())

    assert json.loads(release.calls[0].request.content) == build_release_body(payload())
    assert json.loads(query.calls[0].request.content) == {"task_id_list": ["t-1"]}
    assert audio.calls[0].request.url.params["path"] == "/tmp/t-1_0.wav"
    assert res.audio_path.read_bytes() == wav_bytes(1.5, 48000)
    assert res.seed == 1234 and res.sample_rate == 48000 and res.duration == pytest.approx(1.5)
    assert res.engine_info["task_id"] == "t-1" and res.engine_info["dit_model"] == TURBO
    assert res.engine_info["metas"] == {"bpm": 72, "duration": 30}
    assert any("Diffusion step 3/8" in m for m in ctx.messages)
    assert any("Decoding VAE" in m for m in ctx.messages)
    values = [v for v, _, _ in ctx.events if v > 0.04]
    assert values == sorted(values)


@respx.mock
def test_generate_raw_file_path_and_missing_seed(ace, ctx):
    respx.get(f"{URL}/health").respond(json=HEALTH_OK)
    respx.get(f"{URL}/v1/lora/status").respond(json=lora_status())
    respx.post(f"{URL}/release_task").respond(json=env({"task_id": "t-1"}))
    respx.post(f"{URL}/query_result").mock(return_value=httpx.Response(200, json=env(
        [{"task_id": "t-1", "status": 1, "result": json.dumps([{"file": "/tmp/x.wav", "metas": {}}])}])))
    audio = respx.get(f"{URL}/v1/audio").respond(content=wav_bytes())
    res = ace.generate(ctx, payload())
    assert audio.calls[0].request.url.params["path"] == "/tmp/x.wav"
    assert res.seed == 1234  # the server did not return seed_value -> the seed we sent


@respx.mock
def test_generate_task_failed(ace, ctx):
    respx.get(f"{URL}/health").respond(json=HEALTH_OK)
    respx.get(f"{URL}/v1/lora/status").respond(json=lora_status())
    respx.post(f"{URL}/release_task").respond(json=env({"task_id": "t-3"}))
    respx.post(f"{URL}/query_result").respond(json=env([{"task_id": "t-3", "status": 2, "result": json.dumps(
        [{"file": "", "error": "CUDA out of memory"}])}]))
    with pytest.raises(AceStepError, match="CUDA out of memory") as e:
        ace.generate(ctx, payload())
    assert e.value.retryable is False


@respx.mock
def test_generate_timeout(ace, ctx):
    ace.timeout = 0
    respx.get(f"{URL}/health").respond(json=HEALTH_OK)
    respx.get(f"{URL}/v1/lora/status").respond(json=lora_status())
    respx.post(f"{URL}/release_task").respond(json=env({"task_id": "t-1"}))
    query = respx.post(f"{URL}/query_result").mock(return_value=running(0.1, "Diffusion"))
    audio = respx.get(f"{URL}/v1/audio")
    with pytest.raises(AceStepError, match="did not finish within") as e:
        ace.generate(ctx, payload())
    assert e.value.retryable is False
    assert len(query.calls) == 1 and not audio.called


@respx.mock
def test_generate_when_acestep_down_is_retryable(ace, ctx):
    respx.get(f"{URL}/health").mock(side_effect=httpx.ConnectError("refused"))
    release = respx.post(f"{URL}/release_task")
    with pytest.raises(AceStepUnavailable) as e:
        ace.generate(ctx, payload())
    assert e.value.retryable is True and not release.called


@respx.mock
def test_generate_abnormal_health_is_retryable(ace, ctx):
    respx.get(f"{URL}/health").respond(json=env({"status": "loading"}))
    with pytest.raises(AceStepUnavailable) as e:
        ace.generate(ctx, payload())
    assert e.value.retryable is True


@respx.mock
def test_generate_bad_params_fail_before_touching_acestep(ace, ctx):
    health = respx.get(f"{URL}/health").respond(json=HEALTH_OK)
    with pytest.raises(AceStepError):
        ace.generate(ctx, payload(duration=5000))
    assert not health.called


@respx.mock
def test_generate_connection_lost_while_polling_is_retryable(ace, ctx):
    respx.get(f"{URL}/health").respond(json=HEALTH_OK)
    respx.get(f"{URL}/v1/lora/status").respond(json=lora_status())
    respx.post(f"{URL}/release_task").respond(json=env({"task_id": "t-1"}))
    respx.post(f"{URL}/query_result").mock(side_effect=[running(0.2, "x"), httpx.ReadTimeout("slow")])
    with pytest.raises(AceStepUnavailable) as e:
        ace.generate(ctx, payload())
    assert e.value.retryable is True


@respx.mock
def test_generate_rejects_non_wav_output(ace, ctx):
    respx.get(f"{URL}/health").respond(json=HEALTH_OK)
    respx.get(f"{URL}/v1/lora/status").respond(json=lora_status())
    respx.post(f"{URL}/release_task").respond(json=env({"task_id": "t-1"}))
    respx.post(f"{URL}/query_result").mock(return_value=done())
    respx.get(f"{URL}/v1/audio").respond(content=b"<html>oops</html>")
    with pytest.raises(AceStepError, match="unreadable WAV"):
        ace.generate(ctx, payload())


# ---------------- cancel / lease lost / shutdown ----------------
def _abort_after_first_poll(ctx, attr):
    def hook(c):
        if any("Diffusion step 1/8" in m for m in c.messages):
            setattr(c, attr, True)
    ctx.on_progress = hook


@respx.mock
@pytest.mark.parametrize("attr, exc", [("cancel_requested", JobCancelled), ("lease_lost", LeaseLost)])
def test_abort_mid_task_waits_for_acestep_then_discards(ace, ctx, attr, exc):
    """No cancel API: keep polling until the task ends so the GPU is free, then raise."""
    respx.get(f"{URL}/health").respond(json=HEALTH_OK)
    respx.get(f"{URL}/v1/lora/status").respond(json=lora_status())
    respx.post(f"{URL}/release_task").respond(json=env({"task_id": "t-1"}))
    query = respx.post(f"{URL}/query_result").mock(side_effect=[
        running(0.1, "Diffusion step 1/8"), running(0.5, "Diffusion step 4/8"),
        running(0.9, "Decoding VAE"), done()])
    audio = respx.get(f"{URL}/v1/audio").respond(content=wav_bytes())
    _abort_after_first_poll(ctx, attr)
    with pytest.raises(exc):
        ace.generate(ctx, payload())
    assert len(query.calls) == 4  # polled to the end
    assert not audio.called  # result discarded
    assert any("waiting for the running task" in m for m in ctx.messages)


@respx.mock
def test_cancel_wins_over_task_failure(ace, ctx):
    respx.get(f"{URL}/health").respond(json=HEALTH_OK)
    respx.get(f"{URL}/v1/lora/status").respond(json=lora_status())
    respx.post(f"{URL}/release_task").respond(json=env({"task_id": "t-1"}))
    respx.post(f"{URL}/query_result").mock(side_effect=[
        running(0.1, "Diffusion step 1/8"),
        httpx.Response(200, json=env([{"task_id": "t-1", "status": 2, "result": "[]"}]))])
    _abort_after_first_poll(ctx, "cancel_requested")
    with pytest.raises(JobCancelled):
        ace.generate(ctx, payload())


@respx.mock
def test_cancel_before_submit_never_starts_a_task(ace, ctx):
    respx.get(f"{URL}/health").respond(json=HEALTH_OK)
    respx.get(f"{URL}/v1/lora/status").respond(json=lora_status())
    release = respx.post(f"{URL}/release_task")
    ctx.cancel_requested = True
    with pytest.raises(JobCancelled):
        ace.generate(ctx, payload())
    assert not release.called


@respx.mock
def test_shutdown_mid_task_stops_polling_immediately(ace, ctx):
    respx.get(f"{URL}/health").respond(json=HEALTH_OK)
    respx.get(f"{URL}/v1/lora/status").respond(json=lora_status())
    respx.post(f"{URL}/release_task").respond(json=env({"task_id": "t-1"}))
    query = respx.post(f"{URL}/query_result").mock(side_effect=[
        running(0.1, "Diffusion step 1/8"), running(0.5, "Diffusion step 4/8"), done()])
    _abort_after_first_poll(ctx, "shutdown_requested")
    with pytest.raises(ShutdownRequested):
        ace.generate(ctx, payload())
    assert len(query.calls) == 2


# ---------------- LoRA state ----------------
@respx.mock
def test_lora_job_loads_enables_and_scales(ace, ctx, lora_dir):
    mock_generation_ok()
    respx.get(f"{URL}/v1/lora/status").respond(json=lora_status())
    calls = lora_routes()
    name = adapter_name("chill-turbo")
    res = ace.generate(ctx, payload(lora={"id": "l-1", "key": "chill-turbo", "scale": 0.6}))
    assert calls == [("load", {"lora_path": str((lora_dir / "chill-turbo").resolve()), "adapter_name": name}),
                     ("toggle", {"use_lora": True}),
                     ("scale", {"scale": 0.6, "adapter_name": name})]
    assert res.engine_info["lora_adapter"] == name and res.engine_info["lora_scale"] == 0.6


@respx.mock
def test_lora_already_loaded_is_not_loaded_again(ace, ctx):
    mock_generation_ok()
    respx.get(f"{URL}/v1/lora/status").respond(
        json=lora_status(True, [adapter_name("chill-turbo")], use=False))
    calls = lora_routes()
    ace.generate(ctx, payload(lora={"id": "l-1", "key": "chill-turbo", "scale": 1.7}))
    assert [c[0] for c in calls] == ["toggle", "scale"]
    assert calls[1][1]["scale"] == 1.0  # clamped to the 0..1 range the API accepts


@respx.mock
def test_job_without_lora_disables_active_lora(ace, ctx):
    mock_generation_ok()
    respx.get(f"{URL}/v1/lora/status").respond(json=lora_status(True, [adapter_name("chill-turbo")]))
    calls = lora_routes()
    res = ace.generate(ctx, payload())
    assert calls == [("toggle", {"use_lora": False})]  # never unload
    assert "lora_adapter" not in res.engine_info


@respx.mock
def test_job_without_lora_and_lora_already_off_touches_nothing(ace, ctx):
    mock_generation_ok()
    respx.get(f"{URL}/v1/lora/status").respond(
        json=lora_status(True, [adapter_name("chill-turbo")], use=False))
    calls = lora_routes()
    ace.generate(ctx, payload())
    assert calls == []


@respx.mock
@pytest.mark.parametrize("key", ["missing-lora", "../escape", "not-a-lora"])
def test_lora_not_on_node_fails_without_submitting(ace, ctx, key):
    respx.get(f"{URL}/health").respond(json=HEALTH_OK)
    respx.get(f"{URL}/v1/lora/status").respond(json=lora_status())
    calls = lora_routes()
    release = respx.post(f"{URL}/release_task")
    with pytest.raises(AceStepError) as e:
        ace.generate(ctx, payload(lora={"id": "l", "key": key, "scale": 0.5}))
    assert e.value.retryable is False and calls == [] and not release.called


def test_adapter_names_are_identifier_safe_and_distinct():
    a, b = adapter_name("chill-turbo"), adapter_name("chill_turbo")
    assert a != b and a.replace("_", "").isalnum() and a.startswith("lush_lora_chill_turbo_")


# ---------------- reference audio / cover ----------------
def audio_input(tmp_path, role, name="input.wav", seconds=1.0):
    path = tmp_path / name
    path.write_bytes(wav_bytes(seconds))
    return {role: {"artifact_id": "a-1", "filename": name, "sha256": "x", "size_bytes": 1, "path": str(path)}}


def test_reference_mode_is_text2music_with_the_reference_file(tmp_path):
    p = payload(mode="reference", inputs=audio_input(tmp_path, "reference_audio"))
    body = build_release_body(p)
    assert body == build_release_body(payload())  # same task as plain text; the audio travels as a file
    assert build_release_files(p) == {"reference_audio": tmp_path / "input.wav"}
    assert build_release_files(payload()) == {}


def test_cover_mode_body_and_file(tmp_path):
    p = payload(mode="cover", duration=None, params={"audio_cover_strength": 0.6},
                inputs=audio_input(tmp_path, "source_audio"))
    body = build_release_body(p)
    assert body["task_type"] == "cover" and body["audio_cover_strength"] == 0.6
    assert "audio_duration" not in body  # the length comes from the source audio
    assert build_release_files(p) == {"src_audio": tmp_path / "input.wav"}
    # Strength is a cover-only parameter and stays optional.
    assert "audio_cover_strength" not in build_release_body(payload(params={"audio_cover_strength": 0.6}))
    assert "audio_cover_strength" not in build_release_body(payload(mode="cover", duration=None))


@pytest.mark.parametrize("kw, text", [
    ({"mode": "extend"}, "unknown generation mode"),
    ({"mode": "reference"}, "reference_audio"),
    ({"mode": "cover", "duration": None}, "source_audio"),
    ({"mode": "cover", "duration": None, "params": {"audio_cover_strength": 2}}, "audio_cover_strength"),
])
def test_bad_mode_or_missing_input_fails_without_submitting(ace, ctx, kw, text):
    release = respx.post(f"{URL}/release_task")
    with respx.mock:
        with pytest.raises(Exception) as e:
            ace.generate(ctx, payload(**kw))
    assert text in str(e.value) and getattr(e.value, "retryable", None) is False
    assert not release.called


def test_cover_rejects_source_longer_than_acestep_maximum(tmp_path, monkeypatch):
    import lush_worker.engines.acestep as mod
    monkeypatch.setattr(mod, "probe_duration", lambda path: 601.0)
    with pytest.raises(AceStepError, match="at most 600s"):
        build_release_files(payload(mode="cover", duration=None, inputs=audio_input(tmp_path, "source_audio")))


@respx.mock
@pytest.mark.parametrize("mode, role, field, task", [
    ("reference", "reference_audio", "reference_audio", "text2music"),
    ("cover", "source_audio", "src_audio", "cover"),
])
def test_generate_with_audio_sends_multipart(ace, ctx, tmp_path, mode, role, field, task):
    respx.get(f"{URL}/health").respond(json=HEALTH_OK)
    respx.get(f"{URL}/v1/lora/status").respond(json=lora_status())
    release = respx.post(f"{URL}/release_task").respond(json=env({"task_id": "t-1"}))
    respx.post(f"{URL}/query_result").mock(return_value=done())
    respx.get(f"{URL}/v1/audio").respond(content=wav_bytes(2.0))
    p = payload(mode=mode, inputs=audio_input(tmp_path, role, name="my ref.wav"),
                duration=None if mode == "cover" else 30,
                params={"audio_cover_strength": 0.4} if mode == "cover" else {})

    res = ace.generate(ctx, p)

    request = release.calls[0].request
    assert request.headers["content-type"].startswith("multipart/form-data")
    form = parse_multipart(request)
    assert form[field] == wav_bytes(1.0) and form["_filename"] == "my ref.wav"
    assert form["task_type"] == task and form["prompt"] == "ambient piano, instrumental"
    # Form values are strings; booleans use the spelling the ACE-Step parser understands.
    assert form["thinking"] == "false" and form["use_random_seed"] == "false" and form["seed"] == "1234"
    assert form["audio_format"] == "wav" and form["lyrics"] == "[Instrumental]"
    if mode == "cover":
        assert form["audio_cover_strength"] == "0.4" and "audio_duration" not in form
        assert res.engine_info["audio_cover_strength"] == 0.4
    else:
        assert form["audio_duration"] == "30.0" and "audio_cover_strength" not in form
    assert res.engine_info["task_type"] == task
    assert res.duration == pytest.approx(2.0)
    assert any(f"{field}=my ref.wav" in m for m in ctx.messages)
