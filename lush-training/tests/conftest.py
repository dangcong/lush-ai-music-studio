import json
from pathlib import Path

import numpy as np
import pytest
import soundfile as sf

from lush_training.config import Project

RATE = 16000


def song(seed: int, seconds: float = 20.0, rate: int = RATE) -> np.ndarray:
    """A deterministic stereo test signal: a random note every half second plus a little noise."""
    rng = np.random.default_rng(seed)
    n = int(seconds * rate)
    t = np.arange(n) / rate
    out = np.zeros(n, dtype=np.float32)
    step = rate // 2
    for i in range(0, n, step):
        freq = float(rng.choice([110, 147, 196, 262, 330, 440, 587, 784, 1047, 1568, 2093]))
        seg = slice(i, min(n, i + step))
        out[seg] = 0.3 * np.sin(2 * np.pi * freq * t[seg]) * np.linspace(1.0, 0.2, seg.stop - seg.start)
    out += 0.01 * rng.standard_normal(n).astype(np.float32)
    return np.stack([out, out * 0.9], axis=1)


def write_audio(path: Path, data: np.ndarray, rate: int = RATE, **kw) -> Path:
    path.parent.mkdir(parents=True, exist_ok=True)
    sf.write(str(path), data, rate, **kw)
    return path


@pytest.fixture
def project(tmp_path: Path) -> Project:
    root = tmp_path / "data-root"
    root.mkdir()
    p = Project(data_root=root, acestep_repo=None, acestep_commit=None, acestep_python=None, raw={})
    p.ensure_layout()
    return p


@pytest.fixture
def music(tmp_path: Path) -> Path:
    """Eight distinct test songs."""
    d = tmp_path / "music"
    for i in range(8):
        write_audio(d / f"song {i:02d}.wav", song(i))
    return d


def dataset_cfg(music: Path, **over) -> dict:
    from lush_training.dataset import DEFAULTS
    cfg = {"name": "testset", "source_dirs": [str(music)],
           "rights": {"confirmed": True, "confirmed_by": "Tester", "license": "test"},
           "vocal_flag_threshold": 0.5,
           "segment": {**DEFAULTS["segment"], "seconds": 5.0, "min_seconds": 3.0},
           "split": {**DEFAULTS["split"], "validation": 0.2, "test": 0.2},
           "checks": {**DEFAULTS["checks"], "min_duration": 5.0}}
    cfg.update(over)
    return cfg


def make_run(project: Project, name: str = "run1", **sections) -> Path:
    from lush_training import run as runs
    cfg = {key: (dict(v) if isinstance(v, dict) else v) for key, v in runs.DEFAULTS.items()}
    cfg["limits"] = {"max_steps": 20, "max_epochs": None, "max_hours": None, **sections.get("limits", {})}
    cfg["checkpoint"] = {"every_steps": 5, "every_minutes": 0, "keep_resumable": 2, "milestone_every_steps": 0,
                         **sections.get("checkpoint", {})}
    cfg["validation"] = {"every_steps": 0, "seed": 1, "early_stopping_patience": None, **sections.get("validation", {})}
    run_dir = project.dir("runs") / name
    run_dir.mkdir(parents=True)
    (run_dir / "run.json").write_text(json.dumps({
        "run_id": name, "config": cfg, "training": {"method": "full_decoder", "trainable_parameters": 1,
                                                    "total_parameters": 2}}), encoding="utf-8")
    return run_dir
