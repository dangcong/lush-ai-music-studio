"""Autopilot end to end with a fake backend and synthetic audio. No GPU, no real model."""
from pathlib import Path

import pytest
import yaml
from conftest import song, write_audio
from test_model import make_checkpoint

from lush_training import autopilot, captions, checkpoint, dataset, export
from lush_training.util import LushTrainingError, read_json

TESTS = Path(__file__).parent


def job_file(tmp_path, music, **over):
    job = {"name": "autojob", "authorized_by": "Cong",
           "authorization": {"rights_confirmed": True, "auto_approve_captions": True, "use_gpu": True},
           "music_dir": str(music),
           "captions": {"default": "lo-fi instrumental, mellow keys",
                        "by_folder": {"jazz": "jazz trio instrumental, upright bass, brushed drums"}},
           "segment": {"seconds": 10, "min_seconds": 5},
           "model": {"checkpoint": "acestep-v15-base", "license": "MIT (khai báo)"},
           "method": "full_decoder",
           "training": {"optimizer": {"type": "adamw", "learning_rate": 1e-5}, "gradient_accumulation": 2},
           "stop": {"epochs": 4}}
    job.update(over)
    path = tmp_path / "job.yaml"
    path.write_text(yaml.safe_dump(job, allow_unicode=True), encoding="utf-8")
    return path


@pytest.fixture
def library(tmp_path, project, monkeypatch):
    music = tmp_path / "library"
    for i in range(6):
        write_audio(music / "lofi" / f"lofi {i}.wav", song(i, 35.0))
    for i in range(4):
        write_audio(music / "jazz" / f"jazz {i}.wav", song(50 + i, 35.0))
    make_checkpoint(project.checkpoints_dir / "acestep-v15-base")
    for dep in export.SHARED_DEPENDENCIES:
        (project.checkpoints_dir / dep).mkdir()
    monkeypatch.setenv("TRAINING_DATA_ROOT", str(project.data_root))
    monkeypatch.setenv("PYTHONPATH", str(TESTS))
    return music


def pilot(project, path, tmp_path):
    return autopilot.Autopilot(project, path, project_path=tmp_path / "none.yaml", backend="fake_backend:ExportableFake",
                               preprocess=lambda run_dir: 0, log=lambda _msg: None)


def test_job_runs_from_folder_to_package_and_reruns_are_idempotent(project, library, tmp_path):
    path = job_file(tmp_path, library)
    result = pilot(project, path, tmp_path).run()
    assert result["finished"] is True
    state = read_json(project.data_root / "autopilot" / "autojob" / "state.json")
    stages = state["stages"]

    vdir = dataset.version_dir(project, "autojob", stages["dataset"]["version"])
    manifest, _ = dataset.load_manifest(vdir)
    assert manifest["totals"]["test"]["segments"] == 0 and manifest["totals"]["validation"]["segments"] > 0
    assert manifest["rights"]["confirmed_by"] == "Cong" and manifest["vocal_screening"]["method"] == "none"

    locked = read_json(vdir / "captions" / "auto" / "captions.json")
    track_path = {t["track_id"]: t["source_path"].replace("\\", "/") for t in manifest["tracks"]}
    segment_track = {s["segment_id"]: s["track_id"] for s in manifest["segments"]}
    for cap in locked:
        in_jazz = "/jazz/" in track_path[segment_track[cap["segment_id"]]]
        assert cap["caption_text"].startswith("jazz trio" if in_jazz else "lo-fi instrumental")
        edit = cap["human_edits"][-1]
        assert cap["status"] == "approved" and "autopilot" in edit["by"] and "chưa ai nghe" in edit["note"]
    assert captions.load_lock(vdir, "auto")["segments"]["train"] == stages["captions"]["segments"]["train"]

    run_dir = Path(stages["run"]["run_dir"])
    spe = stages["run"]["steps_per_epoch"]
    assert stages["run"]["max_steps"] == 4 * spe and stages["train"]["step"] == 4 * spe
    assert stages["train"]["stop_reason"] in ("max_steps", "max_epochs")
    selected = stages["export"]["selected"]
    assert selected["step"] == 2 * spe and selected["validation_loss"] == 0.3     # lowest validation loss, not the last one
    assert read_json(checkpoint.step_dir(run_dir, 2 * spe) / "MANIFEST.json")["kind"] == "milestone"
    package = Path(result["package"])
    assert package.name == f"autojob-step{2 * spe}" and export.verify_package(package) == []
    assert read_json(package / "MANIFEST.json")["evaluated"] is False
    report = (project.data_root / "autopilot" / "autojob" / "REPORT.md").read_text(encoding="utf-8")
    assert "ĐÃ XONG" in report and "KHÔNG có bước sàng lọc" in report and "không phải kết quả nghe" in report

    again = pilot(project, path, tmp_path).run()                                   # nothing is redone or duplicated
    assert again["package"] == result["package"] and again["finished"] is True
    assert len(list(project.dir("runs").iterdir())) == 1
    assert len([p for p in (project.dir("datasets") / "autojob").iterdir() if p.name != "_work"]) == 1


def test_changed_library_or_descriptions_do_not_silently_alter_a_started_job(project, library, tmp_path):
    path = job_file(tmp_path, library)
    pilot(project, path, tmp_path).run()
    changed = job_file(tmp_path, library, captions={"default": "ambient instrumental"})
    with pytest.raises(LushTrainingError, match="mô tả trong job file đã đổi"):
        pilot(project, changed, tmp_path).run()
    write_audio(library / "lofi" / "new song.wav", song(99, 35.0))
    with pytest.raises(LushTrainingError, match="kho nhạc"):
        pilot(project, path, tmp_path).run()


def test_trainer_failure_stops_the_pipeline_without_export(project, library, tmp_path, monkeypatch):
    path = job_file(tmp_path, library)
    monkeypatch.setenv("FAKE_NAN_AT", "3")
    with pytest.raises(LushTrainingError, match="trainer dừng vì lỗi"):
        pilot(project, path, tmp_path).run()
    assert not any(project.dir("exports").iterdir())
    with pytest.raises(LushTrainingError, match="không tự"):
        pilot(project, path, tmp_path).run()


@pytest.mark.parametrize("change, message", [
    ({"authorization": {"rights_confirmed": True, "auto_approve_captions": True, "use_gpu": False}}, "use_gpu"),
    ({"authorized_by": ""}, "authorized_by"),
    ({"captions": {}}, "captions.default"),
    ({"captions": {"default": "soulful female vocals over piano"}}, "vocal"),
    ({"stop": {}}, "stop"),
    ({"training": {"optimizer": {"type": "adamw"}}}, "learning_rate"),
    ({"surprise": 1}, "khóa không hợp lệ"),
])
def test_job_file_must_carry_explicit_authorization_and_inputs(tmp_path, change, message):
    with pytest.raises(LushTrainingError, match=message):
        autopilot.load_job(job_file(tmp_path, tmp_path, **change))


def test_time_only_job_uses_a_constant_schedule(project, library, tmp_path):
    path = job_file(tmp_path, library, stop={"hours": 0.1, "total_train_hours": 5})       # shorter than the export reserve: no training happens
    with pytest.raises(LushTrainingError, match="chưa có checkpoint"):
        pilot(project, path, tmp_path).run()
    cfg = yaml.safe_load((project.data_root / "autopilot" / "autojob" / "training.yaml").read_text(encoding="utf-8"))
    assert cfg["scheduler"]["type"] == "constant" and cfg["limits"]["max_epochs"] is None


def test_stop_limits_reach_the_run_config(project, library, tmp_path):
    path = job_file(tmp_path, library, stop={"epochs": 4, "total_train_hours": 6, "patience_epochs": 1, "at": "06:00"})
    result = pilot(project, path, tmp_path).run()
    stages = read_json(project.data_root / "autopilot" / "autojob" / "state.json")["stages"]
    cfg = read_json(Path(result["run_dir"]) / "run.json")["config"]
    assert cfg["limits"]["max_hours"] == 6 and cfg["validation"]["early_stopping_patience"] == 1
    # validation losses 0.5, 0.3, 0.4: one epoch without improvement -> stops after epoch 3, exports epoch 2
    spe = stages["run"]["steps_per_epoch"]
    assert stages["train"]["stop_reason"] == "early_stopping" and stages["train"]["step"] == 3 * spe
    assert stages["export"]["selected"]["step"] == 2 * spe
    assert "early_stopping" in (Path(result["run_dir"]) / "PROGRESS.md").read_text(encoding="utf-8")
    with pytest.raises(LushTrainingError, match="điểm kết thúc"):
        autopilot.load_job(job_file(tmp_path, library, stop={"hours": 8}))
