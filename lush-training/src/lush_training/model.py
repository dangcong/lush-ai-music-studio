"""Inspect a pretrained checkpoint and decide which training backend, if any, can train it.

Every number is read from the checkpoint itself (safetensors header + config.json). Nothing is
hardcoded per variant: a checkpoint whose structure does not match a backend is reported as
incompatible, it is never assumed to be "the same as turbo".
"""
from __future__ import annotations

import re
from collections import defaultdict
from pathlib import Path
from typing import Any, Optional

from . import safetensors_io
from .util import LushTrainingError, read_json, sha256_file

WEIGHTS_FILE = "model.safetensors"
NAME_OK = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$")

# What each training method updates. Names are used verbatim in run.json and in reports.
TRAINING_METHODS = {
    "full_decoder": "Full fine-tuning toàn bộ DiT decoder. Condition encoder, tokenizer, detokenizer, VAE và "
                    "text encoder đóng băng. KHÔNG phải cập nhật toàn bộ model.",
    "partial_decoder": "Partial fine-tuning: chỉ các tensor của DiT decoder khớp `trainable_patterns`.",
    "full_model": "Full fine-tuning toàn bộ model (mọi thành phần). Chưa có backend.",
    "lora": "LoRA/PEFT adapter. Không thuộc workspace này (Train Style của Lush Studio đã làm việc đó).",
}
IMPLEMENTED_METHODS = ("full_decoder", "partial_decoder")


def component_of(name: str) -> str:
    return name.split(".", 1)[0]


def inspect_checkpoint(model_dir: Path, *, with_hash: bool = False) -> dict[str, Any]:
    """Structure of one model directory (`config.json` + `model.safetensors` + engine assets)."""
    model_dir = Path(model_dir)
    if not model_dir.is_dir():
        raise LushTrainingError(f"Không có thư mục checkpoint: {model_dir}")
    weights = model_dir / WEIGHTS_FILE
    if not weights.is_file():
        raise LushTrainingError(f"{model_dir}: không có {WEIGHTS_FILE}")
    header = safetensors_io.read_header(weights)
    components: dict[str, dict[str, Any]] = defaultdict(lambda: {"tensors": 0, "parameters": 0, "bytes": 0,
                                                                  "dtypes": set()})
    for info in header.tensors.values():
        c = components[component_of(info.name)]
        c["tensors"] += 1
        c["parameters"] += info.count
        c["bytes"] += info.nbytes
        c["dtypes"].add(info.dtype)
    config_path = model_dir / "config.json"
    config = read_json(config_path) if config_path.is_file() else None
    report = {
        "model_dir": str(model_dir),
        "name": model_dir.name,
        "weights_file": WEIGHTS_FILE,
        "weights_bytes": header.file_bytes,
        "weights_sha256": sha256_file(weights) if with_hash else None,
        "total_parameters": sum(c["parameters"] for c in components.values()),
        "total_tensors": len(header.tensors),
        "components": {k: {**v, "dtypes": sorted(v["dtypes"])} for k, v in sorted(components.items())},
        "config": None if config is None else {
            "architectures": config.get("architectures"),
            "model_type": config.get("model_type"),
            "auto_map": config.get("auto_map"),
            "is_turbo": config.get("is_turbo"),
            "timestep_mu": config.get("timestep_mu"),
            "timestep_sigma": config.get("timestep_sigma"),
            "data_proportion": config.get("data_proportion"),
        },
        "files": sorted(p.name for p in model_dir.iterdir() if p.is_file()),
        "safetensors_metadata": header.metadata,
    }
    report["compatibility"] = check_compatibility(report)
    return report


def check_compatibility(report: dict[str, Any]) -> dict[str, Any]:
    """Which backend can train this checkpoint. `acestep15-dit` is the only backend in this workspace."""
    reasons: list[str] = []
    warnings: list[str] = []
    components = report["components"]
    config = report["config"]
    if config is None:
        reasons.append("thiếu config.json")
    else:
        arch = " ".join(config.get("architectures") or []) + " " + str(config.get("auto_map") or "")
        if "acestep" not in arch.lower().replace("-", "").replace("_", ""):
            reasons.append("config.json không khai báo kiến trúc ACE-Step (architectures/auto_map)")
        if not any(f.startswith("modeling_") and f.endswith(".py") for f in report["files"]):
            reasons.append("thiếu file modeling_*.py mà engine nạp bằng trust_remote_code")
        if config.get("is_turbo"):
            warnings.append("checkpoint turbo (distilled): upstream train LoRA trên turbo, nhưng full fine-tuning "
                            "một model distilled chưa có bằng chứng; ưu tiên variant base")
    if "decoder" not in components:
        reasons.append("không có thành phần `decoder` trong weights")
    for name, c in components.items():
        if any(d not in ("F32", "BF16", "F16") for d in c["dtypes"]) and name == "decoder":
            reasons.append(f"decoder có dtype không phải số thực: {c['dtypes']} (model lượng tử hóa không train được)")
    if "silence_latent.pt" not in report["files"]:
        warnings.append("thiếu silence_latent.pt: gói inference xuất ra sẽ thiếu asset này")
    ok = not reasons
    return {
        "backend": "acestep15-dit" if ok else None,
        "compatible": ok,
        "methods": list(IMPLEMENTED_METHODS) if ok else [],
        "reasons": reasons,
        "warnings": warnings,
        "note": "Tương thích về cấu trúc file. Chưa nói gì về việc có vừa VRAM của máy này hay không: phải đo.",
    }


def trainable_plan(report: dict[str, Any], method: str, patterns: Optional[list[str]] = None,
                   tensor_names: Optional[dict[str, int]] = None) -> dict[str, Any]:
    """Trainable / frozen split a run would use, computed from the checkpoint header."""
    if method not in TRAINING_METHODS:
        raise LushTrainingError(f"training method không hợp lệ: {method!r}. Chọn một trong {sorted(TRAINING_METHODS)}")
    if method not in IMPLEMENTED_METHODS:
        raise LushTrainingError(f"`{method}`: {TRAINING_METHODS[method]} Workspace này không tự đổi sang method khác.")
    components = report["components"]
    total = report["total_parameters"]
    if method == "full_decoder":
        trainable = components["decoder"]["parameters"]
        trainable_components = ["decoder"]
    else:
        if not patterns:
            raise LushTrainingError("partial_decoder cần `trainable_patterns` (regex trên tên tensor của decoder)")
        if tensor_names is None:
            raise LushTrainingError("partial_decoder cần danh sách tensor của checkpoint")
        compiled = [re.compile(p) for p in patterns]
        matched = {n: c for n, c in tensor_names.items()
                   if component_of(n) == "decoder" and any(r.search(n) for r in compiled)}
        if not matched:
            raise LushTrainingError("trainable_patterns không khớp tensor nào của decoder")
        trainable = sum(matched.values())
        trainable_components = ["decoder (một phần)"]
    return {
        "method": method,
        "description": TRAINING_METHODS[method],
        "trainable_components": trainable_components,
        "frozen_components": sorted(c for c in components if c != "decoder")
        + ["vae (file riêng)", "text encoder (file riêng)"],
        "trainable_parameters": trainable,
        "total_parameters": total,
        "trainable_fraction_of_model_file": round(trainable / total, 6) if total else None,
    }


def tensor_counts(model_dir: Path) -> dict[str, int]:
    header = safetensors_io.read_header(Path(model_dir) / WEIGHTS_FILE)
    return {n: t.count for n, t in header.tensors.items()}
