"""Style LoRA training with a fake trainer process (tests/fake_sidestep.py) and a fake inference
service. Nothing here touches a GPU: what it proves is the worker's handling of the trainer's output,
of failures, and of the inference server's stop / start around a run."""
from __future__ import annotations

import hashlib
import json
import sys
import wave
from pathlib import Path
from typing import Any, Optional

import httpx
import pytest

from lush_worker import audio, training
from lush_worker.api_client import ControlPlaneClient
from lush_worker.config import Config, load_config
from lush_worker.engines.base import JobCancelled, LeaseLost, ShutdownRequested
from lush_worker.engines.mock import MockEngine
from lush_worker.runner import Runner
from lush_worker.training import (InferenceService, SideStepTrainer, TrainingError, build_snapshot_dataset,
                                  build_trainer, sha256_file)

from .conftest import BASE, CP, JOB_ID, LEASE, REGISTER_OK, TOKEN, FastEvent

FAKE = Path(__file__).with_name("fake_sidestep.py")
MODEL = "acestep-v15-xl-sft"


class FakeService:
    unit = "acestep-api"

    def __init__(self, restart_ok: bool = True):
        self.calls: list[str] = []
        self.restart_ok = restart_ok
        self.active = True

    def is_active(self) -> bool:
        return self.active

    def stop(self) -> None:
        self.calls.append("stop")
        self.active = False

    def start(self) -> bool:
        self.calls.append("start")
        self.active = self.restart_ok
        return self.restart_ok


class Ctx:
    def __init__(self, job_id: str = JOB_ID):
        self.job_id = job_id
        self.reports: list[dict[str, Any]] = []
        self.cancel_requested = False
        self.lease_lost = False
        self.shutdown_requested = False
        self.cancel_at_step: Optional[int] = None

    def report(self, value, message=None, stage="running", phase=None, metrics=None):
        self.reports.append({"value": value, "message": message, "phase": phase, "metrics": metrics})
        if self.cancel_at_step and any(m["global_step"] >= self.cancel_at_step for m in metrics or []):
            self.cancel_requested = True

    def raise_if_aborted(self):
        if self.lease_lost:
            raise LeaseLost("lease lost")
        if self.cancel_requested:
            raise JobCancelled("cancelled")
        if self.shutdown_requested:
            raise ShutdownRequested("shutdown")

    @property
    def metrics(self) -> list[dict[str, Any]]:
        return [m for r in self.reports for m in r["metrics"] or []]

    @property
    def phases(self) -> list[str]:
        out: list[str] = []
        for r in self.reports:
            if r["phase"] and (not out or out[-1] != r["phase"]):
                out.append(r["phase"])
        return out


def payload(lora_key: str = "lofi-jazz-0001", **config: Any) -> dict[str, Any]:
    cfg = {"rank": 32, "alpha": 64, "learning_rate": 1e-4, "epochs": 4, "gradient_accumulation": 4,
           "save_every": 2, "warmup_steps": 2, "seed": 42}
    cfg.update(config)
    return {"lora": {"key": lora_key, "name": "Lo-fi Jazz"}, "dataset": {"key": "jazz/v1"},
            "base_model": {"id": "m-1", "engine": "acestep", "key": MODEL}, "trainer": "sidestep",
            "config": cfg, "limits": {"max_seconds": 600}}


@pytest.fixture
def node(tmp_path) -> dict[str, Path]:
    """A render node's disk: ACE-Step checkout, one dataset version of 8 samples, LORA_DIR."""
    ace, data, loras = tmp_path / "ACE-Step-1.5", tmp_path / "data", tmp_path / "data" / "loras"
    (ace / "checkpoints" / MODEL).mkdir(parents=True)
    dataset = data / "datasets" / "jazz" / "v1"
    (dataset / "audio").mkdir(parents=True)
    samples = [{"filename": f"t{i}.wav", "caption": "lo-fi", "duration": 60} for i in range(8)]
    (dataset / "dataset.json").write_text(json.dumps({"metadata": {"name": "jazz"}, "samples": samples}))
    loras.mkdir(parents=True)
    return {"ace": ace, "data": data, "loras": loras}


@pytest.fixture
def service() -> FakeService:
    return FakeService()


@pytest.fixture
def trainer(node, service) -> SideStepTrainer:
    return SideStepTrainer(node["ace"], node["data"], node["loras"], service,  # type: ignore[arg-type]
                           gpu_query=lambda: {"vram_used_mb": 5300}, python=Path(sys.executable), script=FAKE)


def test_lists_dataset_versions_on_disk(trainer, node):
    (node["data"] / "datasets" / "broken" / "v1").mkdir(parents=True)
    (node["data"] / "datasets" / "broken" / "v1" / "dataset.json").write_text("{not json")
    assert trainer.list_datasets() == [{"key": "jazz/v1", "name": "jazz", "samples": 8, "seconds": 480.0}]


def test_trainer_gets_the_checkpoint_folder_name_as_variant(trainer, node, service):
    """ACE-Step ca1e85f: `train.py fixed` only maps turbo/base/sft aliases (cli/args.py VARIANT_DIR_MAP), so
    `xl_sft` fails with "Model directory not found .../checkpoints/xl_sft". The folder name works in every
    step (literal-folder fallback), and timestep parameters still come from that folder's config.json."""
    seen: list[list[str]] = []
    real = trainer._spawn

    def spy(ctx, cmd, *a, **kw):
        seen.append(cmd)
        return real(ctx, cmd, *a, **kw)
    trainer._spawn = spy
    trainer._load_adapter = lambda key, path: None
    trainer.run(Ctx(), payload())
    variants = [c[c.index("--model-variant") + 1] for c in seen]
    assert variants == [MODEL, MODEL]


def test_full_run_reports_real_metrics_and_publishes_the_adapter(trainer, node, service):
    loaded: list[tuple[str, Path]] = []
    trainer._load_adapter = lambda key, path: loaded.append((key, path))
    ctx = Ctx()
    result = trainer.run(ctx, payload())

    # Inference is stopped before anything else and started again afterwards.
    assert service.calls == ["stop", "start"]
    assert ctx.phases == ["stopping_inference", "preprocessing", "training", "saving", "restoring_inference"]
    # 8 samples / accumulation 4 = 2 optimizer steps per epoch, 4 epochs.
    assert [m["global_step"] for m in ctx.metrics] == list(range(1, 9))
    assert [m["sequence"] for m in ctx.metrics] == list(range(1, 9))
    assert ctx.metrics[0] == {"sequence": 1, "global_step": 1, "epoch": 1, "train_loss": 0.89,
                              "learning_rate": None, "elapsed_seconds": ctx.metrics[0]["elapsed_seconds"],
                              "gpu_memory_mb": 5300}
    assert ctx.metrics[-1]["epoch"] == 4
    assert result["epochs_completed"] == 4 and result["optimizer_steps"] == 8
    assert result["stop_reason"] == "completed"
    assert result["inference_restored"] is True and result["adapter_loadable"] is True
    assert result["vram_peak_mb"] == 5300
    assert [c["epoch"] for c in result["checkpoints"]] == [2, 4]
    assert "preprocess_seconds" in result and "train_seconds" in result

    target = node["loras"] / "lofi-jazz-0001"
    assert sorted(p.name for p in target.iterdir()) == ["adapter_config.json", "adapter_model.safetensors"]
    assert result["lora"] == {"key": "lofi-jazz-0001", "source": "final", "bytes": len(b"weights:final"),
                              "adapter_sha256": sha256_file(target / "adapter_model.safetensors")}
    assert loaded == [("lofi-jazz-0001", target)]
    run_dir = node["data"] / "training" / JOB_ID
    assert json.loads((run_dir / "run.json").read_text())["result"]["lora"]["key"] == "lofi-jazz-0001"
    assert "Step 8" in (run_dir / "train.log").read_text()
    # Epoch and checkpoint lines reach the job log; per-step lines travel only as metrics.
    messages = [r["message"] for r in ctx.reports if r["message"]]
    assert "[OK] Checkpoint saved at epoch 2" in messages
    assert not any("Step" in m for m in messages)


def test_tensors_are_preprocessed_once_per_dataset_and_model(trainer, node):
    trainer.run(Ctx(), payload())
    ctx = Ctx("22222222-2222-2222-2222-222222222222")
    result = trainer.run(ctx, payload("second-0002"))
    assert "preprocess_seconds" not in result
    assert any("already preprocessed" in (r["message"] or "") for r in ctx.reports)
    assert (node["loras"] / "second-0002").is_dir()


def test_trainer_runs_from_the_common_parent_directory(trainer, node):
    """ACE-Step's tensor loader rejects paths outside its working directory (path_safety.py)."""
    assert trainer.cwd == node["ace"].parent.resolve()
    cmd = trainer.train_cmd("xl_sft", Path("/t"), Path("/o"), payload()["config"])
    assert cmd[1] == str(FAKE) and "--preprocess" not in cmd
    assert cmd[cmd.index("--lr") + 1] == "0.0001" and cmd[cmd.index("--rank") + 1] == "32"
    assert "--preprocess" in trainer.preprocess_cmd("xl_sft", Path("/d"), Path("/t"), Path("/o"))


@pytest.mark.parametrize("mode, expected", [
    ("nan", "loss is not finite at optimizer step 2"),
    ("oom", "out of GPU memory"),
    ("crash", "exit code 1"),
    ("preprocess_fail", "Preprocessing failed: boom"),
])
def test_failures_restore_inference_and_publish_nothing(trainer, node, service, monkeypatch, mode, expected):
    monkeypatch.setenv("FAKE_MODE", mode)
    ctx = Ctx()
    with pytest.raises(TrainingError, match=expected) as info:
        trainer.run(ctx, payload())
    assert info.value.retryable is False
    assert service.calls == ["stop", "start"]
    assert list(node["loras"].iterdir()) == []
    if mode == "nan":  # the non-finite value is never reported as a metric
        assert [m["global_step"] for m in ctx.metrics] == [1]


def test_time_limit_keeps_the_last_complete_checkpoint(trainer, node, service, monkeypatch):
    monkeypatch.setenv("FAKE_MODE", "slow")  # 0.4 s per optimizer step
    body = payload(epochs=50, save_every=1)
    body["limits"] = {"max_seconds": 2.5}
    result = trainer.run(Ctx(), body)
    assert result["stop_reason"] == "time_limit"
    assert result["lora"]["source"].startswith("epoch_")
    assert 1 <= result["epochs_completed"] < 50
    assert service.calls == ["stop", "start"]
    assert (node["loras"] / "lofi-jazz-0001" / "adapter_model.safetensors").is_file()


def test_time_limit_before_any_checkpoint_fails(trainer, node, service, monkeypatch):
    monkeypatch.setenv("FAKE_MODE", "slow")
    body = payload(epochs=50, save_every=50)
    body["limits"] = {"max_seconds": 1.5}
    with pytest.raises(TrainingError, match="before the first checkpoint"):
        trainer.run(Ctx(), body)
    assert service.calls == ["stop", "start"]
    assert list(node["loras"].iterdir()) == []


def test_cancel_stops_the_trainer_and_restores_inference(trainer, node, service, monkeypatch):
    monkeypatch.setenv("FAKE_MODE", "slow")
    ctx = Ctx()
    ctx.cancel_at_step = 1
    with pytest.raises(JobCancelled):
        trainer.run(ctx, payload(epochs=50))
    assert service.calls == ["stop", "start"]
    assert list(node["loras"].iterdir()) == []


def test_never_overwrites_an_existing_lora_and_does_not_stop_inference(trainer, node, service):
    (node["loras"] / "lofi-jazz-0001").mkdir()
    with pytest.raises(TrainingError, match="never overwritten"):
        trainer.run(Ctx(), payload())
    assert service.calls == []


@pytest.mark.parametrize("change, expected", [
    ({"dataset": {"key": "../etc"}}, "invalid dataset key"),
    ({"dataset": {"key": "missing/v9"}}, "not on this node"),
    ({"base_model": {"key": "mock-v1"}}, "no trainer for base model"),
    ({"lora": {"key": "bad key!"}}, "invalid LoRA key"),
    ({"config": {"rank": 0}}, "invalid training config|out of range"),
])
def test_bad_payloads_fail_before_inference_is_stopped(trainer, service, change, expected):
    with pytest.raises(TrainingError, match=expected):
        trainer.run(Ctx(), {**payload(), **change})
    assert service.calls == []


def test_adapter_that_inference_cannot_load_is_not_registered(trainer, node):
    def broken(key, path):
        raise RuntimeError("shape mismatch")
    trainer._load_adapter = broken
    with pytest.raises(TrainingError, match="could not be loaded by inference: shape mismatch"):
        trainer.run(Ctx(), payload())
    assert list(node["loras"].iterdir()) == []


def test_run_completes_but_says_so_when_inference_does_not_come_back(node):
    service = FakeService(restart_ok=False)
    trainer = SideStepTrainer(node["ace"], node["data"], node["loras"], service,  # type: ignore[arg-type]
                              load_adapter=lambda key, path: pytest.fail("must not load"),
                              python=Path(sys.executable), script=FAKE)
    result = trainer.run(Ctx(), payload())
    assert result["inference_restored"] is False and "adapter_loadable" not in result


def test_recover_starts_inference_left_stopped_by_a_crash(trainer, service):
    service.active = False
    trainer.recover()
    assert service.calls == ["start"]
    trainer.recover()  # already running: nothing to do
    assert service.calls == ["start"]


# ---------- InferenceService ----------

class Proc:
    def __init__(self, stdout: str = "", returncode: int = 0):
        self.stdout, self.stderr, self.returncode = stdout, "", returncode


def test_service_stop_waits_until_the_gpu_is_free():
    calls: list[list[str]] = []
    pids = iter([[4242], [4242], []])
    def run(cmd, **kw):
        calls.append(cmd[2:])
        return Proc("inactive\n" if cmd[2] == "is-active" else "")
    svc = InferenceService("acestep-api", ready=lambda: True, run=run, compute_pids=lambda: next(pids),
                           sleep=lambda s: None)
    svc.stop()
    assert calls[0] == ["stop", "acestep-api"]
    assert next(pids, "exhausted") == "exhausted"


def test_service_stop_fails_when_the_gpu_stays_busy():
    svc = InferenceService("acestep-api", ready=lambda: True, run=lambda cmd, **kw: Proc("inactive\n"),
                           compute_pids=lambda: [4242], sleep=lambda s: None)
    with pytest.raises(TrainingError, match="GPU still in use"):
        svc.stop(timeout=0.0)


def test_service_start_waits_for_the_model_and_reports_a_server_that_never_gets_ready():
    answers = iter([RuntimeError("connection refused"), False, True])
    def ready():
        value = next(answers)
        if isinstance(value, Exception):
            raise value
        return value
    svc = InferenceService("acestep-api", ready=ready, run=lambda cmd, **kw: Proc(), sleep=lambda s: None)
    assert svc.start() is True
    never = InferenceService("acestep-api", ready=lambda: False, run=lambda cmd, **kw: Proc(), sleep=lambda s: None)
    assert never.start(timeout=0.0) is False


# ---------- configuration / runner ----------

def test_training_is_off_unless_fully_configured(tmp_path):
    env = {"CONTROL_PLANE_URL": CP, "NODE_TOKEN": TOKEN, "WORKER_ENGINES": "acestep"}
    cfg = load_config(env, env_file=None)
    assert build_trainer(cfg, {"acestep": object()}) is None
    full = load_config({**env, "ACESTEP_DIR": str(tmp_path / "ace"), "DATA_DIR": str(tmp_path / "data"),
                        "LORA_DIR": str(tmp_path / "data" / "loras")}, env_file=None)
    assert full.acestep_service == "acestep-api"
    assert build_trainer(full, {}) is None  # no ACE-Step engine on this node


def train_job() -> dict[str, Any]:
    return {"id": JOB_ID, "type": "train_lora", "attempt": 1, "lease_token": LEASE,
            "lease_expires_at": "2026-01-01T00:00:00.000Z", "payload": payload()}


@pytest.fixture
def train_runner(tmp_path, trainer) -> Runner:
    cfg = Config(control_plane_url=CP, node_token=TOKEN, engines=("mock",), work_dir=tmp_path / "work",
                 mock_step_delay=0.0)
    r = Runner(cfg, {"mock": MockEngine(step_delay=0.0)}, ControlPlaneClient(CP, TOKEN),
               gpu_query=lambda: None, trainer=trainer)
    r._stop = FastEvent()
    r._retry_delay = 0.0
    return r


def test_registration_advertises_training_and_datasets(train_runner):
    body = train_runner.registration(fresh_start=True)
    assert body["job_types"] == ["generate", "train_lora", "lora_files"]
    assert body["datasets"] == [{"key": "jazz/v1", "name": "jazz", "samples": 8, "seconds": 480.0}]


@pytest.mark.respx(assert_all_called=False)
def test_runner_trains_completes_and_registers_again(train_runner, respx_mock, node):
    respx_mock.post(f"{BASE}/jobs/{JOB_ID}/progress").mock(
        return_value=httpx.Response(200, json={"cancel_requested": False}))
    complete = respx_mock.post(f"{BASE}/jobs/{JOB_ID}/complete").mock(
        return_value=httpx.Response(200, json={"ok": True}))
    register = respx_mock.post(f"{BASE}/register").mock(return_value=httpx.Response(200, json=REGISTER_OK))
    train_runner.run_job(train_job())

    result = json.loads(complete.calls.last.request.content)["result"]
    assert result["lora"]["key"] == "lofi-jazz-0001" and result["optimizer_steps"] == 8
    progress = [json.loads(c.request.content) for c in respx_mock.calls
                if c.request.url.path.endswith("/progress")]
    assert {p.get("phase") for p in progress} >= {"training", "restoring_inference"}
    steps = [m["global_step"] for p in progress for m in p.get("metrics") or []]
    assert steps == list(range(1, 9))
    # The new LoRA is reported straight away instead of at the next periodic registration.
    assert json.loads(register.calls.last.request.content)["fresh_start"] is False
    assert (node["loras"] / "lofi-jazz-0001").is_dir()


@pytest.mark.respx(assert_all_called=False)
def test_runner_never_requeues_a_training_run_on_shutdown(train_runner, respx_mock, monkeypatch):
    monkeypatch.setenv("FAKE_MODE", "slow")
    def on_progress(request):
        if b"global_step" in request.content:
            train_runner.stop()
        return httpx.Response(200, json={"cancel_requested": False})
    respx_mock.post(f"{BASE}/jobs/{JOB_ID}/progress").mock(side_effect=on_progress)
    fail = respx_mock.post(f"{BASE}/jobs/{JOB_ID}/fail").mock(
        return_value=httpx.Response(200, json={"status": "failed"}))
    release = respx_mock.post(f"{BASE}/jobs/{JOB_ID}/release").mock(return_value=httpx.Response(200, json={}))
    respx_mock.post(f"{BASE}/register").mock(return_value=httpx.Response(200, json=REGISTER_OK))
    job = train_job()
    job["payload"]["config"]["epochs"] = 50
    train_runner.run_job(job)
    body = json.loads(fail.calls.last.request.content)
    assert "stopped during training" in body["error"] and body["retryable"] is False
    assert not release.called


# ---------------------------------------------------------------- snapshot datasets (Task 4)

def fake_convert(src: Path, dst: Path) -> None:
    with wave.open(str(dst), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(48000)
        w.writeframes(b"\x00\x00" * 2 * 4800)  # 0.1 s


def snapshot_payload(tmp_path: Path, n: int = 2, bpm=None, keyscale=None) -> dict[str, Any]:
    src = tmp_path / "src"
    src.mkdir(exist_ok=True)
    inputs, samples = {}, []
    for i in range(n):
        f = src / f"in{i}.mp3"
        f.write_bytes(b"x" * (i + 1))
        inputs[f"t{i}"] = {"artifact_id": f"a{i}", "filename": f.name, "size_bytes": i + 1,
                           "sha256": hashlib.sha256(f.read_bytes()).hexdigest(), "path": str(f)}
        samples.append({"input": f"t{i}", "train_caption": f"lushlofi, track {i}",
                        "bpm": (bpm or [None] * n)[i], "keyscale": (keyscale or [None] * n)[i],
                        "duration_s": 180})
    p = payload()
    p["dataset"] = {"key": "lofi-jazz-0001", "snapshot_id": "snap-1"}
    p.update({"genre": "Lo-fi", "samples": samples, "inputs": inputs})
    return p


def test_snapshot_dataset_is_built_in_side_step_format(tmp_path):
    payload = snapshot_payload(tmp_path, n=2, bpm=[80, None], keyscale=["A minor", None])
    count = build_snapshot_dataset(tmp_path / "datasets" / "k1", payload, convert=fake_convert)
    data = json.loads((tmp_path / "datasets" / "k1" / "dataset.json").read_text())
    assert count == 2
    assert data["metadata"] == {"name": "lofi-jazz-0001", "custom_tag": "", "tag_position": "prepend",
                                "all_instrumental": True, "genre_ratio": 0}
    s0, s1 = data["samples"]
    assert s0["audio_path"] == "s000.wav" and s0["caption"].startswith("lushlofi, ")
    assert s0["bpm"] == 80 and s0["keyscale"] == "A minor" and s0["lyrics"] == "[Instrumental]"
    assert s0["genre"] == "Lo-fi" and s0["is_instrumental"] is True and s0["labeled"] is True
    assert s0["language"] == "unknown" and s0["duration"] == 0
    assert "bpm" not in s1 and "keyscale" not in s1
    assert (tmp_path / "datasets" / "k1" / "audio" / "s001.wav").is_file()
    assert not (tmp_path / "datasets" / "k1.partial").exists()


def test_snapshot_dataset_caps_duration_at_240_seconds(tmp_path, monkeypatch):
    monkeypatch.setattr(training, "probe_wav", lambda p: (48000, 500.0))
    build_snapshot_dataset(tmp_path / "d", snapshot_payload(tmp_path, n=1), convert=fake_convert)
    assert json.loads((tmp_path / "d" / "dataset.json").read_text())["samples"][0]["duration"] == 240


def test_snapshot_dataset_failure_leaves_nothing(tmp_path):
    def boom(src, dst): raise RuntimeError("ffmpeg not found")
    with pytest.raises(TrainingError, match="ffmpeg not found"):
        build_snapshot_dataset(tmp_path / "datasets" / "k1", snapshot_payload(tmp_path, n=1), convert=boom)
    assert not (tmp_path / "datasets" / "k1").exists()
    assert not (tmp_path / "datasets" / "k1.partial").exists()


def test_snapshot_dataset_is_never_overwritten(tmp_path):
    (tmp_path / "k1").mkdir()
    with pytest.raises(TrainingError, match="already exists on this node; it is never overwritten"):
        build_snapshot_dataset(tmp_path / "k1", snapshot_payload(tmp_path, n=1), convert=fake_convert)


def test_snapshot_dataset_stops_when_aborted_between_files(tmp_path):
    ctx = Ctx()
    calls = []
    def conv(s, d):
        calls.append(s); fake_convert(s, d); ctx.cancel_requested = True
    with pytest.raises(JobCancelled):
        build_snapshot_dataset(tmp_path / "k1", snapshot_payload(tmp_path, n=3), convert=conv, ctx=ctx)
    assert len(calls) == 1 and not (tmp_path / "k1.partial").exists()


def test_to_wav48k_without_ffmpeg_raises(monkeypatch, tmp_path):
    monkeypatch.setattr(audio, "ffmpeg_path", lambda: None)
    with pytest.raises(RuntimeError, match="ffmpeg"):
        audio.to_wav48k(tmp_path / "a.mp3", tmp_path / "a.wav")


def test_to_wav48k_failure_includes_stderr(monkeypatch, tmp_path):
    class P: returncode, stderr = 1, "bad input " * 100
    monkeypatch.setattr(audio, "ffmpeg_path", lambda: "ffmpeg")
    monkeypatch.setattr(audio.subprocess, "run", lambda *a, **k: P())
    with pytest.raises(RuntimeError, match="bad input") as e:
        audio.to_wav48k(tmp_path / "a.mp3", tmp_path / "a.wav")
    assert len(str(e.value)) < 400


def test_snapshot_run_fails_before_stopping_inference_when_audio_breaks(trainer, service, monkeypatch, tmp_path):
    monkeypatch.setattr(training, "to_wav48k", lambda s, d: (_ for _ in ()).throw(RuntimeError("ffmpeg not found")))
    with pytest.raises(TrainingError):
        trainer.run(Ctx(), snapshot_payload(tmp_path, n=1))
    assert "stop" not in service.calls


def test_snapshot_run_trains_with_fake_sidestep(trainer, node, service, tmp_path, monkeypatch):
    monkeypatch.setattr(training, "to_wav48k", fake_convert)
    ctx = Ctx()
    result = trainer.run(ctx, snapshot_payload(tmp_path, n=2))
    assert result["samples"] == 2
    assert (node["data"] / "datasets" / "lofi-jazz-0001" / "dataset.json").is_file()
    assert ctx.phases[0] == "preparing_dataset"


@pytest.mark.respx(assert_all_called=False)
def test_runner_downloads_snapshot_inputs_before_training(train_runner, respx_mock, monkeypatch, tmp_path):
    seen = {}
    def fake_run(ctx, payload):
        seen["bytes"] = Path(payload["inputs"]["t0"]["path"]).read_bytes()
        return {"lora": {"key": "x"}}
    monkeypatch.setattr(train_runner.trainer, "run", fake_run)
    respx_mock.post(f"{BASE}/jobs/{JOB_ID}/progress").mock(
        return_value=httpx.Response(200, json={"cancel_requested": False}))
    complete = respx_mock.post(f"{BASE}/jobs/{JOB_ID}/complete").mock(return_value=httpx.Response(200, json={"ok": True}))
    respx_mock.post(f"{BASE}/register").mock(return_value=httpx.Response(200, json=REGISTER_OK))
    p = snapshot_payload(tmp_path, n=1)
    p["inputs"] = {"t0": {"artifact_id": "a0", "filename": "in0.mp3", "size_bytes": 3,
                          "sha256": hashlib.sha256(b"abc").hexdigest()}}
    respx_mock.get(f"{BASE}/jobs/{JOB_ID}/inputs/a0").mock(return_value=httpx.Response(200, content=b"abc"))
    job = train_job(); job["payload"] = p
    train_runner.run_job(job)
    assert seen["bytes"] == b"abc"
    assert complete.called


@pytest.mark.respx(assert_all_called=False)
def test_runner_snapshot_bad_sha_fails_without_training(train_runner, respx_mock, monkeypatch, tmp_path):
    ran = []
    monkeypatch.setattr(train_runner.trainer, "run", lambda c, p: ran.append(1) or {})
    respx_mock.post(f"{BASE}/jobs/{JOB_ID}/progress").mock(
        return_value=httpx.Response(200, json={"cancel_requested": False}))
    fail = respx_mock.post(f"{BASE}/jobs/{JOB_ID}/fail").mock(return_value=httpx.Response(200, json={"status": "failed"}))
    respx_mock.post(f"{BASE}/register").mock(return_value=httpx.Response(200, json=REGISTER_OK))
    p = snapshot_payload(tmp_path, n=1)
    p["inputs"] = {"t0": {"artifact_id": "a0", "filename": "in0.mp3", "size_bytes": 3,
                          "sha256": hashlib.sha256(b"abc").hexdigest()}}
    respx_mock.get(f"{BASE}/jobs/{JOB_ID}/inputs/a0").mock(return_value=httpx.Response(200, content=b"zzz"))
    job = train_job(); job["payload"] = p
    train_runner.run_job(job)
    assert fail.called and not ran


# ---------------------------------------------------------------- full checkpoint report (Task 5)

def test_result_lists_every_checkpoint_with_step_and_sha(trainer, node, service):
    result = trainer.run(Ctx(), payload(epochs=4, save_every=1))
    cps = result["checkpoints"]
    assert [c["epoch"] for c in cps] == [1, 2, 3, 4]
    # 8 samples / accumulation 4 = 2 optimizer steps per epoch.
    assert [c["step"] for c in cps] == [2, 4, 6, 8]
    assert all(len(c["sha256"]) == 64 and c["path"] and c["bytes"] > 0 for c in cps)
    for c in cps:
        p = Path(c["path"])
        assert p.is_absolute() and p.is_dir()
        assert c["sha256"] == sha256_file(p / "adapter_model.safetensors")
        assert c["bytes"] == sum(f.stat().st_size for f in p.iterdir() if f.is_file())


def test_final_adapter_is_reported_when_last_epoch_has_no_checkpoint(trainer, node, service):
    result = trainer.run(Ctx(), payload(epochs=3, save_every=2))
    cps = result["checkpoints"]
    assert [c["epoch"] for c in cps] == [2, 3]
    assert cps[-1]["path"].endswith("final") and Path(cps[-1]["path"]).is_absolute()
    assert cps[-1]["step"] == 6
    assert cps[-1]["sha256"] == result["lora"]["adapter_sha256"]


def test_final_is_not_duplicated_when_last_epoch_has_a_checkpoint(trainer, node, service):
    result = trainer.run(Ctx(), payload(epochs=4, save_every=2))
    assert [c["epoch"] for c in result["checkpoints"]] == [2, 4]
    assert not any(c["path"].endswith("final") for c in result["checkpoints"])


def test_completed_run_whose_log_matched_no_epoch_still_reports_final(trainer, node, service, monkeypatch):
    monkeypatch.setenv("FAKE_MODE", "silent")  # exits 0 and writes out/final, prints no epoch/checkpoint lines
    result = trainer.run(Ctx(), payload(epochs=3, save_every=2))
    assert result["stop_reason"] == "completed"
    cps = result["checkpoints"]
    assert [c["epoch"] for c in cps] == [3]
    assert cps[0]["path"].endswith("final")


def test_time_limit_reports_only_existing_checkpoints(trainer, node, service, monkeypatch):
    monkeypatch.setenv("FAKE_MODE", "slow")
    body = payload(epochs=50, save_every=1)
    body["limits"] = {"max_seconds": 2.5}
    result = trainer.run(Ctx(), body)
    cps = result["checkpoints"]
    assert result["stop_reason"] == "time_limit" and cps
    assert not any(c["path"].endswith("final") for c in cps)
    assert all(c["step"] is not None and len(c["sha256"]) == 64 for c in cps)


def test_snapshot_run_with_existing_lora_builds_no_dataset_and_keeps_inference(trainer, node, service, tmp_path,
                                                                              monkeypatch):
    monkeypatch.setattr(training, "to_wav48k", fake_convert)
    (node["loras"] / "lofi-jazz-0001").mkdir()
    with pytest.raises(TrainingError, match="already exists"):
        trainer.run(Ctx(), snapshot_payload(tmp_path, n=1))
    assert not (node["data"] / "datasets" / "lofi-jazz-0001").exists()
    assert not (node["data"] / "datasets" / "lofi-jazz-0001.partial").exists()
    assert service.calls == []
