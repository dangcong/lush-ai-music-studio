"""eval_lora: listening renders with ACE-Step, mp3 uploads, scores (docs/WORKER_PROTOCOL.md)."""
from __future__ import annotations

import hashlib
import math
import shutil
from pathlib import Path
from typing import Any, Optional

import numpy as np
import pytest
import soundfile as sf

from lush_worker import evaluation
from lush_worker.engines.base import EngineError, GenerateResult, JobCancelled
from lush_worker.scoring import ScorerError
from .conftest import FakeCtx

RATE = 22050
TEST_PROMPT = "lofi hip hop, dusty drums, instrumental, no vocals"
HELD_PROMPT = "rainy window piano, instrumental, no vocals"
WEIGHTS = b"w"

# 3 s test tones are long enough for chroma_cqt: any librosa warning is a test failure.
pytestmark = pytest.mark.filterwarnings("error::UserWarning")


def write_tone(path: Path, freq: float, seconds: float = 3.0) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    t = np.arange(int(RATE * seconds)) / RATE
    sf.write(str(path), 0.3 * np.sin(2 * np.pi * freq * t), RATE)


class FakeEngine:
    name = "acestep"

    def __init__(self, fail_at: Optional[int] = None, on_render=None, events: Optional[list] = None,
                 metas: Optional[list] = None):
        self.metas = metas or []
        self.events = events if events is not None else []
        self.payloads: list[dict[str, Any]] = []
        self.workdirs: list[Path] = []
        self.fail_at = fail_at
        self.on_render = on_render

    def generate(self, ctx, payload):
        i = len(self.payloads)
        self.payloads.append(payload)
        self.workdirs.append(Path(ctx.workdir))
        self.events.append("render")
        if self.fail_at == i:
            raise EngineError("boom")
        ctx.progress(0.5, f"[acestep] half {i}")
        out = Path(ctx.workdir) / "master.wav"
        write_tone(out, 220.0 * (1 + i % 3))
        if self.on_render:
            self.on_render(i, ctx)
        info = {"metas": self.metas[i]} if i < len(self.metas) else {}
        return GenerateResult(audio_path=out, seed=payload["seed"], duration=3.0, sample_rate=RATE, engine_info=info)


class FakeApi:
    def __init__(self):
        self.uploads: list[tuple[str, str, str]] = []

    def upload_artifact(self, job_id, lease_token, kind, path, sha256, content_type):
        self.uploads.append((kind, Path(path).suffix, content_type))
        return {"artifact_id": f"a-{kind}"}


class FakeService:
    unit = "acestep-api"

    def __init__(self, events: Optional[list] = None):
        self.calls: list[str] = []
        self.events = events if events is not None else []

    def stop(self):
        self.calls.append("stop")
        self.events.append("stop")

    def start(self):
        self.calls.append("start")
        self.events.append("start")
        return True


def fake_mp3(wav: Path, dest: Path) -> Path:
    shutil.copyfile(wav, dest)
    return dest


class FakeScorer:
    """Scorer stand-in: `answer(path)` gives the value; counts calls per path."""

    def __init__(self, answer, events: Optional[list] = None):
        self.events = events if events is not None else []
        self.answer = answer
        self.calls: list[str] = []
        self.started = self.closed = False

    def start(self):
        self.started = True

    def score(self, path):
        self.calls.append(str(path))
        self.events.append("score")
        value = self.answer(Path(path))
        if isinstance(value, Exception):
            raise value
        return value

    def close(self):
        self.closed = True


@pytest.fixture
def setup(tmp_path):
    root = tmp_path / "training"
    cks = []
    for epoch in (1, 2):
        d = root / "job-1" / "out" / "checkpoints" / f"epoch_{epoch}_loss_0.1"
        d.mkdir(parents=True)
        (d / "adapter_config.json").write_text("{}")
        (d / "adapter_model.safetensors").write_bytes(WEIGHTS)
        cks.append({"id": f"ck{epoch}", "epoch": epoch, "key": f"chill-e{epoch}", "path": str(d),
                    "sha256": hashlib.sha256(WEIGHTS).hexdigest()})
    datasets = tmp_path / "datasets"
    write_tone(datasets / "chill" / "audio" / "s000.wav", 220.0)
    write_tone(datasets / "chill" / "audio" / "s001.wav", 330.0)
    payload = {
        "node_id": "n-1", "lora": {"id": "l-1", "base_key": "chill"}, "dataset_key": "chill",
        "scale": 0.7, "duration": 60, "seed": 123,
        "render_template": {"prompt": TEST_PROMPT, "duration": 60, "seed": 123, "lora": None, "mode": "text",
                            "model": {"id": "m-1", "engine": "acestep", "key": "acestep-v15-xl-sft"},
                            "params": {"inference_steps": 8}, "output": {"formats": ["wav", "mp3"]}},
        "test_prompt": TEST_PROMPT,
        "checkpoints": cks,
        "train": [{"dataset_file": "s000.wav", "filename": "late_bus.mp3"},
                  {"dataset_file": "s001.wav", "filename": "night_walk.mp3"}],
        "held_out": [{"filename": "rainy_window.mp3", "prompt": HELD_PROMPT}],
    }
    work = tmp_path / "work"
    work.mkdir()
    return {"payload": payload, "root": root, "datasets": datasets, "ctx": FakeCtx(work)}


def run(setup, engine=None, scorers=None, service=None, api=None):
    engine = engine or FakeEngine()
    api = api or FakeApi()
    service = service or FakeService()
    result = evaluation.run_eval(setup["ctx"], setup["payload"], engine=engine, api=api,
                                 dataset_dir=setup["datasets"], scorers=scorers or {}, service=service,
                                 checkpoint_root=setup["root"], to_mp3=fake_mp3)
    return result, engine, api, service


def test_eval_renders_in_order_and_uploads(setup):
    result, engine, api, _ = run(setup)
    assert len(engine.payloads) == 5
    assert [u[0] for u in api.uploads] == ["eval_00", "eval_01", "eval_02", "eval_03", "eval_04"]
    assert all(u[1] == ".mp3" and u[2] == "audio/mpeg" for u in api.uploads)
    p = engine.payloads
    assert "lora" not in p[0] and p[0]["prompt"] == TEST_PROMPT
    assert p[1]["lora"] == {"key": "chill-e1", "scale": 0.7, "path": setup["payload"]["checkpoints"][0]["path"]}
    assert p[1]["prompt"] == TEST_PROMPT
    assert p[2]["prompt"] == HELD_PROMPT and p[2]["lora"]["key"] == "chill-e1"
    assert p[3]["lora"]["key"] == "chill-e2" and p[3]["prompt"] == TEST_PROMPT
    assert p[4]["lora"]["key"] == "chill-e2" and p[4]["prompt"] == HELD_PROMPT
    assert all(x["seed"] == 123 and x["duration"] == 60 for x in p)
    assert all(x["model"]["key"] == "acestep-v15-xl-sft" for x in p)
    assert len(set(engine.workdirs)) == 5  # each render has its own workdir
    assert setup["payload"]["render_template"]["lora"] is None  # template untouched
    renders = result["renders"]
    assert [(r["artifact_kind"], r["kind"], r["checkpoint_id"], r["sample_filename"]) for r in renders] == [
        ("eval_00", "base", None, None), ("eval_01", "prompt", "ck1", None),
        ("eval_02", "held_out", "ck1", "rainy_window.mp3"), ("eval_03", "prompt", "ck2", None),
        ("eval_04", "held_out", "ck2", "rainy_window.mp3")]
    assert [r["prompt"] for r in renders] == [TEST_PROMPT, TEST_PROMPT, HELD_PROMPT, TEST_PROMPT, HELD_PROMPT]


def test_eval_progress_reports_rendering_then_scoring(setup):
    run(setup)
    reports = setup["ctx"].reports
    starts = [(v, m) for v, m, s, ph in reports if m and m.startswith("Tạo nhạc nghe thử") and ":" not in m]
    assert [m for _, m in starts] == [f"Tạo nhạc nghe thử {i}/5" for i in range(1, 6)]
    assert [v for v, _ in starts] == pytest.approx([i / 5 * evaluation.RENDER_SHARE for i in range(5)])
    assert all(s == "running" for _, _, s, _ in reports)
    phases = [ph for *_, ph in reports]
    assert "rendering" in phases and "scoring" in phases
    assert phases.index("scoring") > max(i for i, ph in enumerate(phases) if ph == "rendering")
    values = [v for v, *_ in reports]
    assert values == sorted(values) and values[-1] <= 1.0


def test_eval_without_scorers_has_melody_only(setup):
    pytest.importorskip("librosa")
    result, *_ = run(setup)
    assert result["scorers"] == {"mert": False, "audiobox": False}
    for ck in result["checkpoints"]:
        assert isinstance(ck["melody"], float) and 0.0 <= ck["melody"] <= 1.0
        assert ck["melody_match"] in ("late_bus.mp3", "night_walk.mp3")
        assert ck["style"] is None and ck["train"] is None and ck["train_match"] is None and ck["quality"] is None
    assert [c["id"] for c in result["checkpoints"]] == ["ck1", "ck2"]
    for r in result["renders"]:
        s = r["scores"]
        assert set(s) == {"mert_style", "mert_train", "train_match", "melody", "melody_match", "quality"}
        assert isinstance(s["melody"], float)
        assert s["mert_style"] is None and s["mert_train"] is None and s["train_match"] is None
        assert s["quality"] is None


def test_eval_checkpoint_melody_is_max_over_its_renders(setup, monkeypatch):
    # chroma -> a tag; melody_similarity -> a table, so the aggregation is exact.
    monkeypatch.setattr(evaluation, "chroma", lambda path, seconds=60.0: str(path))
    table = {0: (0.1, 0.2), 1: (0.3, 0.6), 2: (0.5, 0.4), 3: (0.2, 0.1), 4: (0.25, 0.15)}
    engine = FakeEngine()

    def sim(a, b):
        i = engine.workdirs.index(Path(a).parent)
        return table[i][0 if b.endswith("s000.wav") else 1]

    monkeypatch.setattr(evaluation, "melody_similarity", sim)
    result, *_ = run(setup, engine=engine)
    r = result["renders"]
    assert (r[1]["scores"]["melody"], r[1]["scores"]["melody_match"]) == (0.6, "night_walk.mp3")
    assert (r[2]["scores"]["melody"], r[2]["scores"]["melody_match"]) == (0.5, "late_bus.mp3")
    ck1, ck2 = result["checkpoints"]
    assert (ck1["melody"], ck1["melody_match"]) == (0.6, "night_walk.mp3")
    assert (ck2["melody"], ck2["melody_match"]) == (0.25, "late_bus.mp3")


def embeddings_for(engine: FakeEngine):
    def answer(path: Path):
        if path.name == "s000.wav":
            return [1.0, 0.0, 0.0]
        if path.name == "s001.wav":
            return [0.0, 1.0, 0.0]
        i = engine.workdirs.index(path.parent)
        return [[1.0, 1.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 1.0, 0.1]][i]
    return answer


def test_eval_with_fake_scorers_computes_scores(setup):
    engine = FakeEngine()
    mert = FakeScorer(embeddings_for(engine))
    quality = FakeScorer(lambda p: float(5 + engine.workdirs.index(p.parent)))
    result, *_ = run(setup, engine=engine, scorers={"mert": mert, "audiobox": quality})
    assert result["scorers"] == {"mert": True, "audiobox": True}
    assert mert.started and mert.closed and quality.started and quality.closed
    r = result["renders"]
    assert r[1]["scores"]["mert_train"] == pytest.approx(1.0)
    assert r[1]["scores"]["train_match"] == "late_bus.mp3"
    assert r[1]["scores"]["mert_style"] == pytest.approx(1 / math.sqrt(2))
    assert r[0]["scores"]["quality"] == 5.0 and r[3]["scores"]["quality"] == 8.0
    ck1, ck2 = result["checkpoints"]
    assert ck1["train"] == pytest.approx(1.0) and ck1["train_match"] == "late_bus.mp3"
    assert ck2["train"] == pytest.approx(1.0) and ck2["train_match"] == "night_walk.mp3"
    assert ck1["style"] == pytest.approx(1 / math.sqrt(2))
    assert ck1["quality"] == pytest.approx(6.5) and ck2["quality"] == pytest.approx(8.5)
    # each train track is embedded once per job, not once per render
    train_calls = [c for c in mert.calls if Path(c).name.startswith("s0")]
    assert sorted(Path(c).name for c in train_calls) == ["s000.wav", "s001.wav"]
    assert len(mert.calls) == 2 + 5 and len(quality.calls) == 5


def test_eval_train_chroma_computed_once_per_job(setup, monkeypatch):
    calls: list[str] = []

    def fake_chroma(path, seconds=60.0):
        calls.append(Path(path).name)
        return np.ones((12, 4))

    monkeypatch.setattr(evaluation, "chroma", fake_chroma)
    monkeypatch.setattr(evaluation, "melody_similarity", lambda a, b: 0.5)
    run(setup)
    assert calls.count("s000.wav") == 1 and calls.count("s001.wav") == 1
    assert calls.count("master.wav") == 5


def test_eval_scorer_error_makes_that_scorer_unavailable(setup):
    engine = FakeEngine()
    good = embeddings_for(engine)

    def flaky(path: Path):
        if path.name == "master.wav" and engine.workdirs.index(path.parent) == 3:
            return ScorerError("CUDA out of memory")
        return good(path)

    mert = FakeScorer(flaky)
    quality = FakeScorer(lambda p: float("nan") if engine.workdirs.index(p.parent) == 1 else 7.0)
    result, *_ = run(setup, engine=engine, scorers={"mert": mert, "audiobox": quality})
    assert result["scorers"] == {"mert": False, "audiobox": True}
    assert len(mert.calls) == 2 + 4  # stopped asking after the error
    for r in result["renders"]:
        assert r["scores"]["mert_style"] is None and r["scores"]["mert_train"] is None
        assert r["scores"]["train_match"] is None
    for ck in result["checkpoints"]:
        assert ck["style"] is None and ck["train"] is None and ck["train_match"] is None
    assert result["renders"][1]["scores"]["quality"] is None  # non-finite -> missing
    ck1, ck2 = result["checkpoints"]
    assert ck1["quality"] == 7.0 and ck2["quality"] == 7.0


def test_eval_scorer_that_cannot_start_is_unavailable(setup):
    class Broken(FakeScorer):
        def start(self):
            raise OSError("no such venv")

    result, *_ = run(setup, scorers={"mert": Broken(lambda p: [1.0]), "audiobox": FakeScorer(lambda p: 6.0)})
    assert result["scorers"] == {"mert": False, "audiobox": True}


def test_eval_missing_dataset_file_fails_before_rendering(setup):
    (setup["datasets"] / "chill" / "audio" / "s001.wav").unlink()
    engine, service = FakeEngine(), FakeService()
    with pytest.raises(EngineError, match="night_walk.mp3"):
        run(setup, engine=engine, service=service)
    assert engine.payloads == [] and service.calls == []


def test_eval_bad_dataset_file_name_fails_before_rendering(setup):
    setup["payload"]["train"][0]["dataset_file"] = "../../secret.wav"
    engine = FakeEngine()
    with pytest.raises(EngineError):
        run(setup, engine=engine)
    assert engine.payloads == []


def test_eval_checkpoint_outside_training_dir_fails_before_rendering(setup, tmp_path):
    bad = tmp_path / "elsewhere" / "out" / "checkpoints" / "x"
    bad.mkdir(parents=True)
    (bad / "adapter_config.json").write_text("{}")
    setup["payload"]["checkpoints"][1]["path"] = str(bad)
    engine, service = FakeEngine(), FakeService()
    with pytest.raises(EngineError):
        run(setup, engine=engine, service=service)
    assert engine.payloads == [] and service.calls == []


def test_eval_missing_checkpoint_dir_fails_before_rendering(setup):
    shutil.rmtree(setup["payload"]["checkpoints"][0]["path"])
    engine = FakeEngine()
    with pytest.raises(EngineError):
        run(setup, engine=engine)
    assert engine.payloads == []


def test_eval_restarts_inference_after_loading_adapters(setup):
    _, _, _, service = run(setup)
    assert service.calls == ["stop", "start"]


def test_eval_base_render_failure_does_not_restart(setup):
    service = FakeService()
    with pytest.raises(EngineError, match="boom"):
        run(setup, engine=FakeEngine(fail_at=0), service=service)
    assert service.calls == []


def test_eval_engine_failure_after_adapter_still_restarts(setup):
    service = FakeService()
    with pytest.raises(EngineError, match="boom"):
        run(setup, engine=FakeEngine(fail_at=1), service=service)
    assert service.calls == ["stop", "start"]


def test_eval_cancel_between_renders_still_restarts(setup):
    def cancel_after_second(i, ctx):
        if i == 1:
            setup["ctx"].cancel_requested = True

    engine, service, api = FakeEngine(on_render=cancel_after_second), FakeService(), FakeApi()
    with pytest.raises(JobCancelled):
        run(setup, engine=engine, service=service, api=api)
    assert len(engine.payloads) == 2
    assert len(api.uploads) == 1  # render 0 uploaded; render 1 discarded
    assert service.calls == ["stop", "start"]


def test_eval_mp3_conversion_failure_fails_the_job(setup):
    engine, service = FakeEngine(), FakeService()
    with pytest.raises(EngineError, match="MP3"):
        evaluation.run_eval(setup["ctx"], setup["payload"], engine=engine, api=FakeApi(),
                            dataset_dir=setup["datasets"], scorers={}, service=service,
                            checkpoint_root=setup["root"], to_mp3=lambda wav, dest: None)
    assert len(engine.payloads) == 1 and service.calls == []


def test_eval_restart_failure_does_not_hide_the_result(setup):
    class BadService(FakeService):
        def stop(self):
            self.calls.append("stop")
            raise RuntimeError("systemctl failed")

    service = BadService()
    result, *_ = run(setup, service=service)
    assert service.calls == ["stop", "start"] and len(result["renders"]) == 5


# ---------- runner ----------
def test_runner_registers_eval_lora_only_with_trainer_and_acestep(tmp_path):
    from lush_worker.api_client import ControlPlaneClient
    from lush_worker.config import Config
    from lush_worker.engines.mock import MockEngine
    from lush_worker.runner import Runner
    from .conftest import CP, TOKEN

    class T:
        training_dir = tmp_path / "training"
        dataset_dir = tmp_path / "datasets"
        list_datasets = staticmethod(lambda: [])

    cfg = Config(control_plane_url=CP, node_token=TOKEN, engines=("mock",), work_dir=tmp_path / "w")
    api = ControlPlaneClient(CP, TOKEN)
    ace = MockEngine(step_delay=0.0)
    ace.name = "acestep"
    with_both = Runner(cfg, {"acestep": ace}, api, gpu_query=lambda: None, trainer=T())
    assert "eval_lora" in with_both.registration(True)["job_types"]
    no_trainer = Runner(cfg, {"acestep": ace}, api, gpu_query=lambda: None)
    assert "eval_lora" not in no_trainer.registration(True)["job_types"]
    no_acestep = Runner(cfg, {"mock": MockEngine(step_delay=0.0)}, api, gpu_query=lambda: None, trainer=T())
    assert "eval_lora" not in no_acestep.registration(True)["job_types"]


def test_runner_dispatches_eval_lora(tmp_path, monkeypatch):
    import json

    import respx
    from lush_worker.api_client import ControlPlaneClient
    from lush_worker.config import Config
    from lush_worker.engines.mock import MockEngine
    from lush_worker.runner import Runner
    from lush_worker.scoring import ScorerSpec
    from .conftest import BASE, CP, JOB_ID, LEASE, TOKEN

    service = FakeService()

    class T:
        training_dir = tmp_path / "training"
        dataset_dir = tmp_path / "datasets"
        list_datasets = staticmethod(lambda: [])

    T.service = service
    ace = MockEngine(step_delay=0.0)
    ace.name = "acestep"
    specs = {"mert-v1": ScorerSpec("mert-v1", "MERT", "1", ["x"], "embedding", tmp_path),
             "pq": ScorerSpec("pq", "Audiobox", "1", ["x"], "quality", tmp_path)}
    cfg = Config(control_plane_url=CP, node_token=TOKEN, engines=("mock",), work_dir=tmp_path / "w")
    r = Runner(cfg, {"acestep": ace}, ControlPlaneClient(CP, TOKEN), gpu_query=lambda: None, trainer=T(),
               scorers=specs)
    r._retry_delay = 0.0
    seen: dict[str, Any] = {}

    def fake_run_eval(ctx, payload, **kw):
        seen.update(kw, payload=payload)
        kw["upload"]("eval_00", fake_mp3_file(ctx.workdir))
        return {"renders": [], "checkpoints": [], "scorers": {"mert": False, "audiobox": False}}

    def fake_mp3_file(workdir: Path) -> Path:
        p = workdir / "x.mp3"
        p.write_bytes(b"mp3")
        return p

    monkeypatch.setattr(evaluation, "run_eval", fake_run_eval)
    job = {"id": JOB_ID, "type": "eval_lora", "attempt": 1, "lease_token": LEASE, "payload": {"lora": {"id": "l"}}}
    with respx.mock:
        respx.post(f"{BASE}/jobs/{JOB_ID}/progress").respond(json={"cancel_requested": False})
        up = respx.post(f"{BASE}/jobs/{JOB_ID}/artifacts").respond(json={"artifact_id": "a"})
        done = respx.post(f"{BASE}/jobs/{JOB_ID}/complete").respond(json={})
        reg = respx.post(f"{BASE}/register").respond(json={"node_id": "n"})
        r.run_job(job)
    assert done.called and up.called and reg.called
    assert json.loads(done.calls[0].request.content)["result"]["scorers"] == {"mert": False, "audiobox": False}
    assert seen["engine"] is ace and seen["service"] is service
    assert seen["dataset_dir"] == T.dataset_dir and seen["checkpoint_root"] == T.training_dir
    assert set(seen["scorers"]) == {"mert", "audiobox"}
    assert seen["scorers"]["mert"].spec.key == "mert-v1" and seen["scorers"]["audiobox"].spec.key == "pq"


# ---------- fix round 1 ----------
def test_eval_checkpoint_sha_mismatch_fails_before_rendering(setup):
    setup["payload"]["checkpoints"][1]["sha256"] = "f" * 64
    engine, service = FakeEngine(), FakeService()
    with pytest.raises(EngineError, match="epoch 2"):
        run(setup, engine=engine, service=service)
    assert engine.payloads == [] and service.calls == []


def test_eval_checkpoint_without_weights_fails_before_rendering(setup):
    (Path(setup["payload"]["checkpoints"][0]["path"]) / "adapter_model.safetensors").unlink()
    engine = FakeEngine()
    with pytest.raises(EngineError):
        run(setup, engine=engine)
    assert engine.payloads == []


def test_eval_seed_falls_back_to_template(setup):
    setup["payload"]["seed"] = None
    setup["payload"]["render_template"]["seed"] = 77
    _, engine, _, _ = run(setup)
    assert all(p["seed"] == 77 for p in engine.payloads)


def test_eval_stops_inference_before_scoring_and_starts_after(setup):
    events: list[str] = []
    engine = FakeEngine(events=events)
    mert = FakeScorer(embeddings_for(engine), events=events)
    quality = FakeScorer(lambda p: 6.0, events=events)
    run(setup, engine=engine, service=FakeService(events), scorers={"mert": mert, "audiobox": quality})
    assert events == ["render"] * 5 + ["stop"] + ["score"] * (2 + 5 + 5) + ["start"]


def test_eval_cancel_during_scoring_starts_inference_once(setup):
    events: list[str] = []
    engine = FakeEngine(events=events)

    def answer(path: Path):
        setup["ctx"].cancel_requested = True
        return [1.0, 0.0, 0.0]

    service = FakeService(events)
    with pytest.raises(JobCancelled):
        run(setup, engine=engine, service=service, scorers={"mert": FakeScorer(answer, events=events)})
    assert service.calls == ["stop", "start"]
    assert events.index("stop") < events.index("score") < events.index("start")


def test_eval_engine_messages_carry_the_render_prefix(setup):
    run(setup)
    messages = [m for _, m, _, _ in setup["ctx"].reports if m and "[acestep]" in m]
    assert messages == [f"Tạo nhạc nghe thử {i + 1}/5: [acestep] half {i}" for i in range(5)]


def test_eval_pins_the_metas_of_the_first_render_of_each_prompt(setup):
    """The LM fills blank BPM / Key / Time Signature at random; every render of one prompt must use the
    values of that prompt's first render, so the A/B differs only by the LoRA."""
    metas = [{"bpm": 72, "keyscale": "A minor", "timesignature": "4"},   # base, test prompt
             {"bpm": 99, "keyscale": "B major", "timesignature": "3"},   # ck1 test prompt (ignored)
             {"bpm": 64, "keyscale": "D minor", "timesignature": "3"},   # ck1 held-out: first of its prompt
             {}, {}]
    _, engine, _, _ = run(setup, engine=FakeEngine(metas=metas))
    p = engine.payloads
    assert not {"bpm", "key", "time_signature"} & {k for k in p[0] if p[0][k] is not None}
    for i in (1, 3):
        assert (p[i]["bpm"], p[i]["key"], p[i]["time_signature"]) == (72, "A minor", "4")
    assert (p[4]["bpm"], p[4]["key"], p[4]["time_signature"]) == (64, "D minor", "3")


def test_eval_keeps_metas_given_by_the_template(setup):
    setup["payload"]["render_template"].update(bpm=80, key="C major", time_signature="4")
    metas = [{"bpm": 72, "keyscale": "A minor", "timesignature": "3"}] + [{}] * 4
    _, engine, _, _ = run(setup, engine=FakeEngine(metas=metas))
    assert all((x["bpm"], x["key"], x["time_signature"]) == (80, "C major", "4") for x in engine.payloads)
