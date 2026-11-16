"""A tiny deterministic backend for testing the run lifecycle without a GPU.

It is NOT a model and proves nothing about real training. It has the properties the lifecycle tests
need: its loss depends on a weight and on a random stream, so a resume that failed to restore either
one produces a different loss sequence.
"""
import json
import os
import random
import time
from pathlib import Path


class FakeBackend:
    weight_files = ("weights.json",)

    def __init__(self, project, run_dir):
        self.run_dir = Path(run_dir)
        opts_file = self.run_dir / "fake_backend.json"
        self.opts = json.loads(opts_file.read_text()) if opts_file.is_file() else {}
        if os.environ.get("FAKE_NAN_AT"):
            self.opts["nan_at"] = int(os.environ["FAKE_NAN_AT"])
        self.w = 1.0
        self.rng = random.Random(7)
        self.validations = 0

    def setup(self):
        return {"backend": "fake"}

    def steps_per_epoch(self):
        return 4

    def train_step(self, step):
        if self.opts.get("sleep"):
            time.sleep(self.opts["sleep"])
        if step == self.opts.get("nan_at"):
            return {"loss": float("nan"), "lr": 0.1}
        if step == self.opts.get("oom_at"):
            from lush_training.session import OutOfMemory
            raise OutOfMemory("fake OOM")
        noise = self.rng.random()
        loss = self.w * self.w + 0.1 * noise
        self.w -= 0.05 * self.w
        return {"loss": loss, "lr": 0.1}

    def validate(self):
        self.validations += 1
        series = self.opts.get("validation")
        return series[min(self.validations - 1, len(series) - 1)] if series else self.w

    def save_state(self, directory):
        (Path(directory) / "weights.json").write_text(json.dumps({"w": self.w}))
        (Path(directory) / "optimizer.json").write_text(json.dumps({"rng": self.rng.getstate(), "v": self.validations}))

    def load_state(self, directory):
        self.w = json.loads((Path(directory) / "weights.json").read_text())["w"]
        st = json.loads((Path(directory) / "optimizer.json").read_text())
        self.rng.setstate((st["rng"][0], tuple(st["rng"][1]), st["rng"][2]))
        self.validations = st["v"]


class ExportableFake(FakeBackend):
    """Fake backend whose checkpoints carry a `decoder.safetensors`, so the export path can be exercised."""
    weight_files = ("decoder.safetensors",)
    VALIDATION = [0.5, 0.3, 0.4, 0.45]

    def __init__(self, project, run_dir):
        super().__init__(project, run_dir)
        self.project = project
        self.info = json.loads((self.run_dir / "run.json").read_text(encoding="utf-8"))
        self.opts.setdefault("validation", self.VALIDATION)

    def steps_per_epoch(self):
        return self.info["config"]["checkpoint"]["every_steps"]

    def save_state(self, directory):
        import numpy as np

        from lush_training import safetensors_io
        super().save_state(directory)
        parent = self.project.checkpoints_dir / self.info["parent"]["name"] / "model.safetensors"
        header = safetensors_io.read_header(parent)
        arrays = {}
        with open(parent, "rb") as f:
            for name, t in header.tensors.items():
                if name.startswith("decoder."):
                    data = np.frombuffer(safetensors_io.read_tensor_bytes(f, header, t), dtype="<f4").reshape(t.shape)
                    arrays[name] = data + np.float32(1.0 - self.w)
        safetensors_io.write_numpy(Path(directory) / "decoder.safetensors", arrays)
