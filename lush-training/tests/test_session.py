"""Run lifecycle with the fake backend: limits, stop, resume, damaged checkpoints. No GPU involved."""
import json
import os
import subprocess
import sys
import time
from pathlib import Path

import pytest
from conftest import make_run
from fake_backend import FakeBackend

from lush_training import checkpoint, run as runs, session
from lush_training.util import read_jsonl

TESTS = Path(__file__).parent


def go(project, run_dir, **kw):
    return session.run_session(run_dir, FakeBackend(project, run_dir), install_signals=False, **kw)


def losses(run_dir):
    return [(m["step"], m["loss"]) for m in read_jsonl(run_dir / "metrics.jsonl") if m["kind"] == "step"]


def test_runs_to_max_steps_and_checkpoints(project):
    run_dir = make_run(project)
    assert go(project, run_dir) == session.EXIT_COMPLETED
    status = runs.read_status(run_dir)
    assert (status["state"], status["stop_reason"], status["step"]) == ("completed", "max_steps", 20)
    assert checkpoint.list_steps(run_dir) == [15, 20]          # keep_resumable = 2
    assert (run_dir / "LATEST").read_text().strip() == "step-00000020"
    assert status["eta_seconds_to_max_steps"] == 0 and not (run_dir / "trainer.pid").exists()
    assert go(project, run_dir) == session.EXIT_COMPLETED        # nothing left to do, no new checkpoint
    assert len(losses(run_dir)) == 20


def test_eta_only_after_enough_measured_steps(project):
    run_dir = make_run(project, limits={"max_steps": 100})
    (run_dir / "fake_backend.json").write_text(json.dumps({"oom_at": 10}))
    assert go(project, run_dir) == session.EXIT_OOM
    status = runs.read_status(run_dir)
    assert status["eta_seconds_to_max_steps"] is None and status["stop_reason"] == "out_of_memory"


def test_stop_file_saves_a_checkpoint_and_resume_continues_exactly(project):
    reference = make_run(project, "reference")
    go(project, reference)

    run_dir = make_run(project, "interrupted")

    class Stopper(FakeBackend):
        def train_step(self, step):
            if step == 8:
                runs.request_stop(run_dir)     # arrives while step 8 is running
            return super().train_step(step)

    assert session.run_session(run_dir, Stopper(project, run_dir), install_signals=False) == session.EXIT_STOP_REQUESTED
    status = runs.read_status(run_dir)
    assert (status["state"], status["stop_reason"], status["step"]) == ("stopped", "stop_requested", 8)
    assert checkpoint.list_steps(run_dir)[-1] == 8             # the running step finished, then the checkpoint
    assert go(project, run_dir) == session.EXIT_COMPLETED
    assert losses(run_dir) == losses(reference)                # weights AND random stream were restored
    events = [e["event"] for e in read_jsonl(run_dir / "events.jsonl")]
    assert "resumed" in events and events.count("session_start") == 2


def test_session_time_limit_and_stop_at(project):
    run_dir = make_run(project, limits={"max_steps": 1000})
    ticks = iter(range(0, 10**6, 60))                          # every clock() call advances one minute
    assert session.run_session(run_dir, FakeBackend(project, run_dir), session_hours=1.0,
                               clock=lambda: float(next(ticks)), install_signals=False) == session.EXIT_TIME_LIMIT
    status = runs.read_status(run_dir)
    assert status["stop_reason"] == "session_time_limit" and 0 < status["step"] < 1000
    assert checkpoint.latest_valid(run_dir)[0].name == f"step-{status['step']:08d}"
    assert go(project, run_dir, stop_at=time.time() - 1) == session.EXIT_TIME_LIMIT
    assert runs.read_status(run_dir)["stop_reason"] == "stop_at"


def test_total_hours_and_epoch_limits(project):
    run_dir = make_run(project, "epochs", limits={"max_steps": 1000, "max_epochs": 3})
    assert go(project, run_dir) == session.EXIT_COMPLETED
    assert runs.read_status(run_dir)["step"] == 12 and runs.read_status(run_dir)["stop_reason"] == "max_epochs"
    run_dir = make_run(project, "hours", limits={"max_steps": 1000, "max_hours": 0.5})
    ticks = iter(range(0, 10**6, 60))
    session.run_session(run_dir, FakeBackend(project, run_dir), clock=lambda: float(next(ticks)), install_signals=False)
    assert runs.read_status(run_dir)["stop_reason"] == "max_hours"


def test_non_finite_loss_stops_without_a_new_checkpoint(project):
    run_dir = make_run(project)
    (run_dir / "fake_backend.json").write_text(json.dumps({"nan_at": 8}))
    assert go(project, run_dir) == session.EXIT_NON_FINITE
    assert checkpoint.list_steps(run_dir) == [5]               # last good one stays; none at step 7 or 8
    assert runs.read_status(run_dir)["state"] == "failed"
    assert [s for s, _ in losses(run_dir)] == list(range(1, 8))


def test_damaged_latest_checkpoint_falls_back_to_the_previous_one(project):
    run_dir = make_run(project, limits={"max_steps": 10})
    go(project, run_dir)
    assert checkpoint.list_steps(run_dir) == [5, 10]
    (checkpoint.step_dir(run_dir, 10) / "weights.json").write_text('{"w": 123.0}')
    latest, rejected = checkpoint.latest_valid(run_dir)
    assert latest.name == "step-00000005" and "sha256 differs" in str(rejected) or "size differs" in str(rejected)
    backend = FakeBackend(project, run_dir)
    backend.load_state(latest)
    assert backend.w != 123.0


def test_no_valid_checkpoint_is_an_error_never_a_silent_restart(project):
    run_dir = make_run(project, limits={"max_steps": 10})
    go(project, run_dir)
    before = len(losses(run_dir))
    for step in checkpoint.list_steps(run_dir):
        (checkpoint.step_dir(run_dir, step) / "optimizer.json").unlink()
    assert go(project, run_dir) == session.EXIT_NO_VALID_CHECKPOINT
    assert runs.read_status(run_dir)["stop_reason"] == "no_valid_checkpoint"
    assert len(losses(run_dir)) == before                      # not a single step was trained


def test_partial_checkpoint_from_a_crash_is_discarded(project):
    run_dir = make_run(project, limits={"max_steps": 10})
    go(project, run_dir)
    partial = checkpoint.checkpoints_dir(run_dir) / "step-00000012.partial"
    partial.mkdir()
    (partial / "weights.json").write_text("{}")
    assert checkpoint.latest_valid(run_dir)[0].name == "step-00000010"
    go(project, run_dir)
    assert not partial.exists()


def test_checkpoint_is_never_overwritten(project):
    run_dir = make_run(project)
    backend = FakeBackend(project, run_dir)
    checkpoint.save(run_dir, 3, backend.save_state, {"step": 3})
    with pytest.raises(Exception, match="không ghi đè"):
        checkpoint.save(run_dir, 3, backend.save_state, {"step": 3})


def test_retention_keeps_milestone_weights_only(project):
    run_dir = make_run(project, checkpoint={"every_steps": 5, "keep_resumable": 1, "milestone_every_steps": 10})
    go(project, run_dir)
    assert checkpoint.list_steps(run_dir) == [10, 20]
    milestone = checkpoint.step_dir(run_dir, 10)
    assert sorted(p.name for p in milestone.iterdir()) == ["MANIFEST.json", "progress.json", "weights.json"]
    assert checkpoint.verify(milestone) and checkpoint.latest_valid(run_dir)[0].name == "step-00000020"


def test_validation_and_early_stopping(project):
    run_dir = make_run(project, limits={"max_steps": 100}, validation={"every_steps": 2, "early_stopping_patience": 2})
    (run_dir / "fake_backend.json").write_text(json.dumps({"validation": [0.5, 0.4, 0.45, 0.46]}))
    assert go(project, run_dir) == session.EXIT_COMPLETED
    status = runs.read_status(run_dir)
    assert (status["stop_reason"], status["step"]) == ("early_stopping", 8)
    vals = [m for m in read_jsonl(run_dir / "metrics.jsonl") if m["kind"] == "validation"]
    assert [v["improved"] for v in vals] == [True, True, False, False]


def test_status_reports_a_dead_trainer_as_interrupted(project):
    run_dir = make_run(project)
    runs.write_status(run_dir, {"state": "running", "step": 3, "pid": 999999})
    (run_dir / "trainer.pid").write_text("999999")
    d = runs.describe(run_dir)
    assert d["status"]["state"] == "interrupted" and d["trainer_pid"] is None


def test_second_trainer_on_the_same_run_is_refused(project):
    run_dir = make_run(project)
    sleeper = subprocess.Popen([sys.executable, "-c", "import time; time.sleep(30)"])
    try:
        (run_dir / "trainer.pid").write_text(str(sleeper.pid))
        assert go(project, run_dir) == session.EXIT_ALREADY_RUNNING
        assert losses(run_dir) == []
    finally:
        sleeper.kill()


def test_detached_trainer_survives_its_launcher_and_stops_on_request(project, tmp_path):
    """Real processes: `train.py start` returns, the trainer keeps running, `stop` ends it with a checkpoint."""
    run_dir = make_run(project, "detached", limits={"max_steps": 100000}, checkpoint={"every_steps": 50})
    (run_dir / "fake_backend.json").write_text(json.dumps({"sleep": 0.02}))
    env = {**os.environ, "TRAINING_DATA_ROOT": str(project.data_root),
           "PYTHONPATH": os.pathsep.join([str(TESTS), str(TESTS.parent / "src")])}
    cli = [sys.executable, "-m", "lush_training.cli", "train"]
    common = ["--run", str(run_dir), "--project", str(tmp_path / "none.yaml")]
    started = subprocess.run([*cli, "start", *common, "--confirm-gpu", "--backend", "fake_backend:FakeBackend"],
                             env=env, capture_output=True, text=True, encoding="utf-8", timeout=60)
    assert started.returncode == 0, started.stderr
    deadline = time.time() + 30
    while time.time() < deadline and runs.read_status(run_dir).get("step", 0) < 5:
        time.sleep(0.2)
    pid = runs.trainer_pid(run_dir)
    assert pid is not None and runs.read_status(run_dir)["state"] == "running"   # launcher exited, trainer alive
    again = subprocess.run([*cli, "resume", *common, "--confirm-gpu", "--backend", "fake_backend:FakeBackend"],
                           env=env, capture_output=True, text=True, encoding="utf-8", timeout=60)
    assert again.returncode == 2                                                  # no checkpoint yet / already running
    assert subprocess.run([*cli, "stop", *common], env=env, capture_output=True, timeout=60).returncode == 0
    while time.time() < deadline and runs.trainer_pid(run_dir) is not None:
        time.sleep(0.2)
    status = runs.read_status(run_dir)
    assert runs.trainer_pid(run_dir) is None
    assert (status["state"], status["stop_reason"]) == ("stopped", "stop_requested")
    assert checkpoint.latest_valid(run_dir)[0].name == f"step-{status['step']:08d}"
    assert "exit 11" in (run_dir / "trainer.log").read_text(encoding="utf-8", errors="replace")


def test_progress_report_is_written_during_and_after_training(project):
    run_dir = make_run(project, limits={"max_steps": 40, "max_epochs": 10, "max_hours": 2.0},
                       validation={"every_steps": 4, "early_stopping_patience": 3})
    seen = []

    class Watcher(FakeBackend):
        def train_step(self, step):
            report = run_dir / "PROGRESS.md"
            if step == 7 and report.is_file():
                seen.append(report.read_text(encoding="utf-8"))
            return super().train_step(step)

    assert session.run_session(run_dir, Watcher(project, run_dir), session_hours=3.0, install_signals=False) == session.EXIT_COMPLETED
    assert seen and "Trạng thái: running" in seen[0] and "step-00000005" in seen[0]      # updated mid-run, at the checkpoint
    text = (run_dir / "PROGRESS.md").read_text(encoding="utf-8")
    assert "Trạng thái: completed" in text and "Optimizer step: 40 / 40 (100.0%)" in text
    assert "Epoch (lượt qua toàn bộ segment train): 10.00 / 10" in text
    assert "Hết ngân sách 2.0 giờ train cộng dồn" in text and "Hết giờ của phiên này" in text
    assert "Validation loss không cải thiện 3 lần liên tiếp" in text and "thấp nhất" in text
    assert "Hợp lệ mới nhất: step-00000040" in text
