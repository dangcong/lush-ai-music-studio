import json
import os
import subprocess
import sys
from pathlib import Path

import numpy as np
import pytest
from conftest import RATE, dataset_cfg, song, write_audio
from test_model import make_checkpoint

from lush_training import acestep_probe, checkpoint, config, dataset, evaluate, export, run as runs, safetensors_io
from lush_training.util import LushTrainingError, read_json, sha256_file


@pytest.fixture
def trained(project):
    """A run with one checkpoint whose decoder differs from the parent in two of three tensors."""
    parent_dir = project.checkpoints_dir / "acestep-v15-base"
    arrays = make_checkpoint(parent_dir)
    for dep in export.SHARED_DEPENDENCIES:
        (project.checkpoints_dir / dep).mkdir()
    run_dir = project.dir("runs") / "r1"
    run_dir.mkdir(parents=True)
    (run_dir / "run.json").write_text(json.dumps({
        "run_id": "r1", "config": {"limits": {"max_steps": 10}},
        "parent": {"name": "acestep-v15-base", "weights_sha256": sha256_file(parent_dir / "model.safetensors"),
                   "total_parameters": 77, "license": "MIT (khai báo)", "source": "test"},
        "training": {"method": "full_decoder", "trainable_parameters": 56, "total_parameters": 77},
        "dataset": {"name": "d", "version": "v1"}, "code": {"acestep_commit": "ca1e85f", "lush_training_commit": None}}))
    decoder = {k: np.asarray(v, dtype=np.float32) for k, v in arrays.items() if k.startswith("decoder.")}
    decoder["decoder.block.0.weight"] = decoder["decoder.block.0.weight"] + 0.25
    decoder["decoder.block.0.bias"] = decoder["decoder.block.0.bias"] * 2.0

    def write(d):
        safetensors_io.write_numpy(d / "decoder.safetensors", decoder)
    checkpoint.save(run_dir, 10, write, {"step": 10, "train_seconds": 12.5})
    return project, run_dir, arrays, decoder


def test_export_swaps_only_the_decoder(trained):
    project, run_dir, arrays, decoder = trained
    package = export.export_package(project, run_dir, "step-00000010", "lush-lofi-v1")
    assert export.verify_package(package) == []
    model_dir = package / "model" / "lush-lofi-v1"
    assert sorted(p.name for p in model_dir.iterdir()) == ["config.json", "model.safetensors",
                                                           "modeling_acestep_v15_base.py", "silence_latent.pt"]
    header = safetensors_io.read_header(model_dir / "model.safetensors")
    parent_header = safetensors_io.read_header(project.checkpoints_dir / "acestep-v15-base" / "model.safetensors")
    assert {n: (t.dtype, t.shape) for n, t in header.tensors.items()} == {n: (t.dtype, t.shape) for n, t in parent_header.tensors.items()}
    with open(model_dir / "model.safetensors", "rb") as f:
        def tensor(name):
            return np.frombuffer(safetensors_io.read_tensor_bytes(f, header, header.tensors[name]), dtype="<f4")
        assert np.array_equal(tensor("decoder.block.0.weight"), decoder["decoder.block.0.weight"].ravel())
        assert np.array_equal(tensor("encoder.proj.weight"), np.asarray(arrays["encoder.proj.weight"], dtype="<f4").ravel())
    prov = read_json(package / "PROVENANCE.json")
    assert prov["weight_merge"] == {"decoder_tensors": 3, "decoder_tensors_changed": 2, "frozen_tensors": 3,
                                    "frozen_tensors_verified_identical": 3}
    assert prov["training"]["method"] == "full_decoder" and prov["parent"]["weights_sha256"]
    assert [d["included"] for d in prov["requires_on_inference_machine"]] == [False, False]
    assert "CHƯA CÓ" in (package / "EVALUATION.md").read_text(encoding="utf-8")
    assert read_json(package / "MANIFEST.json")["evaluated"] is False
    text = (package / "IMPORT.md").read_text(encoding="utf-8")
    assert "Render Node" in text and "VPS" in text and "Rollback" in text
    done = subprocess.run([sys.executable, str(package / "verify_package.py")], capture_output=True, text=True)
    assert done.returncode == 0 and "OK" in done.stdout
    with pytest.raises(LushTrainingError, match="không ghi đè"):
        export.export_package(project, run_dir, "step-00000010", "lush-lofi-v1")


def test_verify_detects_tampering_missing_and_extra_files(trained):
    project, run_dir, _, _ = trained
    package = export.export_package(project, run_dir, "step-00000010", "pkg")
    weights = package / "model" / "pkg" / "model.safetensors"
    data = bytearray(weights.read_bytes())
    data[-1] ^= 1
    weights.write_bytes(bytes(data))
    assert any("sha256 differs" in p for p in export.verify_package(package))
    (package / "model" / "pkg" / "config.json").unlink()
    (package / "stray.bin").write_bytes(b"x")
    problems = export.verify_package(package)
    assert any("config.json" in p for p in problems) and any("file lạ" in p for p in problems)
    assert subprocess.run([sys.executable, str(package / "verify_package.py")], capture_output=True).returncode == 1


def test_export_refuses_bad_inputs(trained):
    project, run_dir, _, decoder = trained
    (checkpoint.step_dir(run_dir, 10) / "decoder.safetensors").write_bytes(b"corrupt")
    with pytest.raises(LushTrainingError, match="không qua kiểm tra"):
        export.export_package(project, run_dir, "step-00000010", "bad1")

    def nan(d):
        bad = dict(decoder)
        bad["decoder.block.1.weight"] = np.full((4, 4), np.nan, dtype=np.float32)
        safetensors_io.write_numpy(d / "decoder.safetensors", bad)
    checkpoint.save(run_dir, 11, nan, {"step": 11})
    with pytest.raises(LushTrainingError, match="không hữu hạn"):
        export.export_package(project, run_dir, "step-00000011", "bad2")

    def wrong_shape(d):
        bad = dict(decoder)
        bad["decoder.block.1.weight"] = np.zeros((2, 2), dtype=np.float32)
        safetensors_io.write_numpy(d / "decoder.safetensors", bad)
    checkpoint.save(run_dir, 12, wrong_shape, {"step": 12})
    with pytest.raises(LushTrainingError, match="shape"):
        export.export_package(project, run_dir, "step-00000012", "bad3")
    parent = project.checkpoints_dir / "acestep-v15-base" / "silence_latent.pt"
    assert not (project.dir("exports") / "bad3").exists() and parent.is_file()
    (project.checkpoints_dir / "acestep-v15-base" / "model.safetensors").write_bytes(b"replaced")
    with pytest.raises(LushTrainingError, match="không còn khớp"):
        export.export_package(project, run_dir, "step-00000012", "bad4")


def test_evaluation_plan_ab_and_report(project, music, tmp_path):
    cfg = dataset_cfg(music)
    dataset.scan(cfg, project)
    manifest = dataset.build(cfg, project, "v1")
    vdir = dataset.version_dir(project, "testset", "v1")
    prompts = tmp_path / "prompts.yaml"
    prompts.write_text('version: "t1"\nseeds: [1, 2]\nprompts:\n'
                       '  - {id: a, duration: 5, prompt: "lo-fi instrumental"}\n'
                       '  - {id: b, duration: 5, in_style: false, prompt: "orchestral instrumental"}\n', encoding="utf-8")
    eval_dir = tmp_path / "eval"
    plan = evaluate.make_plan(prompts, eval_dir, run_id="r1", checkpoint_name="step-00000010")
    assert len(plan["items"]) == 4 and all(i["lyrics"] == "[Instrumental]" for i in plan["items"])
    train_seg = next(s for s in manifest["segments"] if s["split"] == "train")
    for n, item in enumerate(plan["items"]):
        write_audio(eval_dir / "parent" / item["file"], song(100 + n, 5.0))
        write_audio(eval_dir / "candidate" / item["file"], song(200 + n, 5.0))
    # one candidate render is a copy of a training segment, one is silent
    (eval_dir / "candidate" / plan["items"][0]["file"]).write_bytes((vdir / train_seg["file"]).read_bytes())
    write_audio(eval_dir / "candidate" / plan["items"][1]["file"], np.zeros((RATE * 5, 2), dtype=np.float32))
    bad = [r for r in evaluate.check_side(eval_dir, "candidate", plan) if r["problems"]]
    assert [r["file"] for r in bad] == [plan["items"][1]["file"]]
    memo = evaluate.memorization_screen(eval_dir, vdir)
    assert memo["flagged"] == [plan["items"][0]["id"]]
    ab = evaluate.make_ab(eval_dir, seed=3)
    assert ab == {"pairs": 4, "skipped": 0}
    key = read_json(eval_dir / "ab_key.json")["pairs"]
    assert {frozenset((v["A"], v["B"])) for v in key.values()} == {frozenset(evaluate.SIDES)}
    assert len(list((eval_dir / "ab").glob("pair-*.wav"))) == 8
    report = evaluate.write_report(eval_dir, vdir).read_text(encoding="utf-8")
    assert "chưa ai điền" in report and plan["items"][0]["id"] in report and "Không có detector" in report
    with pytest.raises(LushTrainingError):
        evaluate.make_ab(eval_dir)


def test_training_config_validation(tmp_path):
    example = config.WORKSPACE / "configs" / "training.example.yaml"
    cfg = runs.load_training_config(example)
    assert cfg["method"] == "full_decoder" and cfg["limits"]["max_steps"] == 2000

    def broken(old, new):
        p = tmp_path / "t.yaml"
        p.write_text(example.read_text(encoding="utf-8").replace(old, new), encoding="utf-8")
        with pytest.raises(LushTrainingError):
            runs.load_training_config(p)
    broken("method: full_decoder", "method: best")
    broken("max_steps: 2000", "max_steps:")
    broken("learning_rate: 1.0e-5", "learning_rate: 5")
    broken("type: adamw  ", "type: sgd  ")
    broken("weights: fp32", "weights: bf16")
    broken("seed: 42", "seed: 42\nmystery_option: 1")
    broken("trainable_patterns: []", "trainable_patterns: ['x']")
    assert dataset.load_dataset_config(config.WORKSPACE / "configs" / "dataset.example.yaml")["segment"]["seconds"] == 60


def test_data_root_must_be_configured_and_outside_the_workspace(tmp_path, monkeypatch):
    monkeypatch.delenv(config.ENV_DATA_ROOT, raising=False)
    with pytest.raises(LushTrainingError, match="TRAINING_DATA_ROOT"):
        config.load_project(tmp_path / "missing.yaml")
    monkeypatch.setenv(config.ENV_DATA_ROOT, str(config.WORKSPACE / "data"))
    with pytest.raises(LushTrainingError, match="trong workspace"):
        config.load_project(tmp_path / "missing.yaml")
    monkeypatch.setenv(config.ENV_DATA_ROOT, str(tmp_path))
    assert config.load_project(tmp_path / "missing.yaml").data_root == tmp_path.resolve()


def test_probe_rejects_wrong_or_incomplete_checkouts(tmp_path):
    assert not acestep_probe.probe_repo(tmp_path / "nowhere")["ok"]
    repo = tmp_path / "repo"
    (repo / "acestep" / "training_v2").mkdir(parents=True)
    (repo / "acestep" / "training_v2" / "model_loader.py").write_text("def load_decoder_for_training():\n    pass\n")
    probe = acestep_probe.probe_repo(repo)
    assert not probe["ok"] and any("read_model_config" in p for p in probe["problems"])
    assert any("thiếu train.py" in p for p in probe["problems"])


def test_probe_accepts_the_pinned_upstream_checkout():
    """Runs only when ACESTEP_REPO points at a checkout (source only; no weights, no torch needed)."""
    repo = os.environ.get("ACESTEP_REPO")
    if not repo:
        pytest.skip("ACESTEP_REPO not set")
    probe = acestep_probe.probe_repo(Path(repo))
    assert probe["ok"], probe["problems"]
    assert probe["commit"] == acestep_probe.PINNED_COMMIT
