"""lora_files job (publish / delete) and loading a LoRA checkpoint by path."""
from __future__ import annotations

import hashlib
from pathlib import Path

import pytest

from lush_worker import lora_files
from lush_worker.engines.acestep import AceStepEngine, AceStepError, adapter_name
from lush_worker.engines.base import EngineError
from .conftest import FakeCtx, generate_payload


class FakeLoraClient:
    def __init__(self):
        self.loaded: list[tuple[str, str]] = []

    def health(self):
        return {"status": "ok"}

    def lora_status(self):
        return {"lora_loaded": False, "use_lora": False, "adapters": []}

    def lora_load(self, path, adapter_name):
        self.loaded.append((path, adapter_name))

    def lora_toggle(self, on):
        pass

    def lora_scale(self, scale, adapter_name):
        pass


def make_ckpt(root: Path, name: str, weights: bytes = b"weights") -> Path:
    d = root / "job-1" / "out" / "checkpoints" / name
    d.mkdir(parents=True)
    (d / "adapter_config.json").write_text("{}")
    (d / "adapter_model.safetensors").write_bytes(weights)
    return d


def sha(b: bytes) -> str:
    return hashlib.sha256(b).hexdigest()


@pytest.fixture
def root(tmp_path):
    return tmp_path / "training"


@pytest.fixture
def lora_dir(tmp_path):
    d = tmp_path / "loras"
    d.mkdir()
    return d


@pytest.fixture
def ctx(tmp_path):
    w = tmp_path / "job"
    w.mkdir()
    return FakeCtx(w)


def pub(path, digest, key="chill-e3"):
    return {"node_id": "n", "lora_id": "l", "lora": {"id": "l"}, "op": "publish", "target_key": key,
            "checkpoint": {"id": "c1", "path": str(path), "sha256": digest}}


def dele(*items):
    return {"node_id": "n", "lora_id": "l", "lora": {"id": "l"}, "op": "delete",
            "checkpoints": [{"id": i, "path": str(p)} for i, p in items]}


# ---------- engine: LoRA by path ----------
def engine(root, tmp_path, client):
    return AceStepEngine(client, lora_dir=tmp_path / "loras", checkpoint_root=root, poll_interval=0,
                         sleep=lambda s: None)


def test_lora_path_inside_root_is_loaded(root, tmp_path, ctx):
    ck = make_ckpt(root, "epoch_3_loss_0.1")
    client = FakeLoraClient()
    eng = engine(root, tmp_path, client)
    info = eng._apply_lora_state(ctx, {"lora": {"key": "chill-e3", "scale": 0.7, "path": str(ck)}})
    assert client.loaded == [(str(ck.resolve()), adapter_name("chill-e3"))]
    assert info["lora_adapter"] == adapter_name("chill-e3")


@pytest.mark.parametrize("where", ["outside", "traversal", "no_config", "no_root"])
def test_lora_path_outside_root_is_rejected(root, tmp_path, ctx, where):
    ck = make_ckpt(root, "epoch_3_loss_0.1")
    outside = tmp_path / "elsewhere"
    outside.mkdir()
    (outside / "adapter_config.json").write_text("{}")
    path = {"outside": outside, "traversal": root / ".." / "elsewhere", "no_config": root,
            "no_root": ck}[where]
    client = FakeLoraClient()
    eng = AceStepEngine(client, lora_dir=tmp_path / "loras", checkpoint_root=None if where == "no_root" else root,
                        poll_interval=0, sleep=lambda s: None)
    with pytest.raises(AceStepError, match="outside the training directory"):
        eng._apply_lora_state(ctx, {"lora": {"key": "chill-e3", "scale": 0.7, "path": str(path)}})
    assert client.loaded == []


# ---------- publish ----------
def test_publish_copies_adapter_atomically(root, lora_dir, ctx):
    ck = make_ckpt(root, "epoch_3_loss_0.1")
    res = lora_files.run(pub(ck, sha(b"weights")), ctx, checkpoint_root=root, lora_dir=lora_dir)
    assert res == {"published": {"key": "chill-e3", "sha256": sha(b"weights")}}
    target = lora_dir / "chill-e3"
    assert (target / "adapter_model.safetensors").read_bytes() == b"weights"
    assert (target / "adapter_config.json").is_file()
    assert [p.name for p in lora_dir.iterdir()] == ["chill-e3"]  # no .partial left


def test_publish_same_sha_is_idempotent(root, lora_dir, ctx):
    ck = make_ckpt(root, "epoch_3_loss_0.1")
    p = pub(ck, sha(b"weights"))
    first = lora_files.run(p, ctx, checkpoint_root=root, lora_dir=lora_dir)
    assert lora_files.run(p, ctx, checkpoint_root=root, lora_dir=lora_dir) == first


def test_publish_refuses_different_existing_target(root, lora_dir, ctx):
    ck = make_ckpt(root, "epoch_3_loss_0.1")
    t = lora_dir / "chill-e3"
    t.mkdir()
    (t / "adapter_config.json").write_text("{}")
    (t / "adapter_model.safetensors").write_bytes(b"other")
    with pytest.raises(EngineError):
        lora_files.run(pub(ck, sha(b"weights")), ctx, checkpoint_root=root, lora_dir=lora_dir)
    assert (t / "adapter_model.safetensors").read_bytes() == b"other"


def test_publish_bad_sha_fails_without_target(root, lora_dir, ctx):
    ck = make_ckpt(root, "epoch_3_loss_0.1")
    with pytest.raises(EngineError):
        lora_files.run(pub(ck, "0" * 64), ctx, checkpoint_root=root, lora_dir=lora_dir)
    assert list(lora_dir.iterdir()) == []


def test_publish_outside_root_or_bad_key_fails(root, lora_dir, ctx, tmp_path):
    make_ckpt(root, "epoch_3_loss_0.1")
    out = tmp_path / "x"
    out.mkdir()
    (out / "adapter_config.json").write_text("{}")
    (out / "adapter_model.safetensors").write_bytes(b"weights")
    with pytest.raises(EngineError):
        lora_files.run(pub(out, sha(b"weights")), ctx, checkpoint_root=root, lora_dir=lora_dir)
    ck = root / "job-1" / "out" / "checkpoints" / "epoch_3_loss_0.1"
    with pytest.raises(EngineError):
        lora_files.run(pub(ck, sha(b"weights"), key="../evil"), ctx, checkpoint_root=root, lora_dir=lora_dir)
    assert list(lora_dir.iterdir()) == []


# ---------- delete ----------
def test_delete_removes_dirs_and_reports_missing(root, lora_dir, ctx):
    a = make_ckpt(root, "epoch_1_loss_0.1")
    gone = a.parent / "epoch_2_loss_0.1"
    res = lora_files.run(dele(("c1", a), ("c2", gone)), ctx, checkpoint_root=root, lora_dir=lora_dir)
    assert res == {"deleted": ["c1"], "missing": ["c2"]}
    assert not a.exists()


def test_delete_with_outside_path_deletes_nothing(root, lora_dir, ctx, tmp_path):
    a = make_ckpt(root, "epoch_1_loss_0.1")
    out = tmp_path / "precious"
    out.mkdir()
    with pytest.raises(EngineError):
        lora_files.run(dele(("c1", a), ("c2", out), ("c3", root)), ctx, checkpoint_root=root, lora_dir=lora_dir)
    assert a.exists() and out.exists()


def test_unknown_op_fails(root, lora_dir, ctx):
    with pytest.raises(EngineError):
        lora_files.run({"op": "format"}, ctx, checkpoint_root=root, lora_dir=lora_dir)


# ---------- runner ----------
def test_runner_dispatches_lora_files(tmp_path, root, lora_dir):
    import respx
    from lush_worker.api_client import ControlPlaneClient
    from lush_worker.config import Config
    from lush_worker.engines.mock import MockEngine
    from lush_worker.runner import Runner
    from .conftest import BASE, CP, LEASE, TOKEN, JOB_ID

    class T:
        training_dir = root
    t = T()
    t.lora_dir = lora_dir
    t.list_datasets = lambda: []
    ck = make_ckpt(root, "epoch_3_loss_0.1")
    cfg = Config(control_plane_url=CP, node_token=TOKEN, engines=("mock",), work_dir=tmp_path / "work",
                 mock_step_delay=0.0)
    r = Runner(cfg, {"mock": MockEngine(step_delay=0.0)}, ControlPlaneClient(CP, TOKEN),
               gpu_query=lambda: None, trainer=t)
    assert "lora_files" in r.registration(True)["job_types"]
    job = {"id": JOB_ID, "type": "lora_files", "attempt": 1, "lease_token": LEASE,
           "payload": pub(ck, sha(b"weights"))}
    with respx.mock:
        respx.post(f"{BASE}/jobs/{JOB_ID}/progress").respond(json={"cancel_requested": False})
        done = respx.post(f"{BASE}/jobs/{JOB_ID}/complete").respond(json={})
        respx.post(f"{BASE}/nodes/register").respond(json={"node_id": "n"})
        r.run_job(job)
    import json
    assert json.loads(done.calls[0].request.content)["result"]["published"]["key"] == "chill-e3"
    assert (lora_dir / "chill-e3" / "adapter_config.json").is_file()
