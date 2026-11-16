"""Project configuration: where the data lives and which ACE-Step checkout is used.

Nothing here is tied to one machine. `TRAINING_DATA_ROOT` (environment) wins over `data_root` in
`configs/project.yaml`, so the same workspace runs on any training machine.
"""
from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Optional

import yaml

from .util import LushTrainingError

WORKSPACE = Path(__file__).resolve().parents[2]
DEFAULT_PROJECT_CONFIG = WORKSPACE / "configs" / "project.yaml"
ENV_DATA_ROOT = "TRAINING_DATA_ROOT"

# Sub-directories of TRAINING_DATA_ROOT. Created on demand, never inside git.
LAYOUT = {
    "originals": "Original audio, if you choose to keep it here (read-only for this workspace)",
    "datasets": "Dataset versions: <name>/<version>/ with an immutable manifest",
    "tensors": "Tensors preprocessed by the engine, per dataset version + caption version + checkpoint",
    "pretrained": "Pretrained checkpoints (the engine's `checkpoints` directory)",
    "runs": "Training runs: config, status, metrics, logs, resume checkpoints",
    "eval": "Evaluation renders and reports",
    "exports": "Inference packages ready to copy to the GPU machine of Lush Studio",
    "cache": "Analysis cache (safe to delete)",
}


def load_yaml(path: Path) -> dict[str, Any]:
    path = Path(path)
    if not path.is_file():
        raise LushTrainingError(f"Không tìm thấy file cấu hình: {path}")
    data = yaml.safe_load(path.read_text(encoding="utf-8")) or {}
    if not isinstance(data, dict):
        raise LushTrainingError(f"File cấu hình phải là một mapping YAML: {path}")
    return data


@dataclass(frozen=True)
class Project:
    data_root: Path
    acestep_repo: Optional[Path]
    acestep_commit: Optional[str]
    acestep_python: Optional[Path]
    raw: dict[str, Any]

    def dir(self, name: str) -> Path:
        if name not in LAYOUT:
            raise KeyError(name)
        return self.data_root / name

    @property
    def checkpoints_dir(self) -> Path:
        custom = (self.raw.get("acestep") or {}).get("checkpoints_dir")
        return Path(custom).expanduser() if custom else self.dir("pretrained")

    def ensure_layout(self) -> None:
        for name in LAYOUT:
            self.dir(name).mkdir(parents=True, exist_ok=True)


def load_project(path: Optional[Path] = None, *, require_root: bool = True) -> Project:
    """Read `configs/project.yaml`. A missing file is fine as long as TRAINING_DATA_ROOT is set."""
    path = Path(path) if path else DEFAULT_PROJECT_CONFIG
    raw = load_yaml(path) if path.is_file() else {}
    root = os.environ.get(ENV_DATA_ROOT, "").strip() or str(raw.get("data_root") or "").strip()
    if not root:
        if require_root:
            raise LushTrainingError(
                f"Chưa cấu hình vùng dữ liệu. Đặt biến môi trường {ENV_DATA_ROOT} hoặc `data_root` trong {path} "
                "(chép từ configs/project.example.yaml). Vùng này phải nằm ngoài Git."
            )
        root = "."
    data_root = Path(root).expanduser().resolve()
    try:
        data_root.relative_to(WORKSPACE)
        inside = True
    except ValueError:
        inside = False
    if inside and require_root:
        raise LushTrainingError(
            f"{ENV_DATA_ROOT} ({data_root}) nằm trong workspace Git. Chọn một thư mục bên ngoài repository."
        )
    ace = raw.get("acestep") or {}
    repo = Path(ace["repo_dir"]).expanduser() if ace.get("repo_dir") else None
    python = Path(ace["python"]).expanduser() if ace.get("python") else None
    return Project(data_root=data_root, acestep_repo=repo, acestep_commit=ace.get("commit"),
                   acestep_python=python, raw=raw)


def require(mapping: dict[str, Any], key: str, kind: type | tuple[type, ...], where: str) -> Any:
    if key not in mapping or mapping[key] is None:
        raise LushTrainingError(f"{where}: thiếu `{key}`")
    value = mapping[key]
    if isinstance(value, bool) and kind in (int, float, (int, float)):
        raise LushTrainingError(f"{where}: `{key}` phải là số")
    if not isinstance(value, kind):
        raise LushTrainingError(f"{where}: `{key}` sai kiểu (cần {kind})")
    return value
