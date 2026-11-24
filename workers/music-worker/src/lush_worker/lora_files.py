"""Job `lora_files`: file operations on this node, no GPU. See docs/WORKER_PROTOCOL.md.

`publish` copies a chosen training checkpoint into LORA_DIR/<target_key>; `delete` removes checkpoint
directories. Both only touch paths strictly below the trainer's training directory.
"""
from __future__ import annotations

import logging
import re
import shutil
from pathlib import Path
from typing import Any

from .audio import sha256_file
from .engines.base import EngineError

JOB_TYPE = "lora_files"
ADAPTER_FILES = ("adapter_config.json", "adapter_model.safetensors")
TARGET_KEY_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,127}$")

log = logging.getLogger(__name__)


def checkpoint_dir(path: Any, root: Path) -> Path:
    """Resolve `path`; it must be one of the two shapes the trainer produces below `root`:
    `<job>/out/checkpoints/<name>` or `<job>/out/final`. Raises EngineError otherwise."""
    try:
        resolved = Path(str(path)).resolve()
        rel = resolved.relative_to(root.resolve()).parts
    except (OSError, ValueError, TypeError) as e:
        raise EngineError("checkpoint path is outside the training directory") from e
    if not ((len(rel) == 4 and rel[1:3] == ("out", "checkpoints")) or (len(rel) == 3 and rel[1:] == ("out", "final"))):
        raise EngineError("checkpoint path is outside the training directory")
    return resolved


_under_root = checkpoint_dir


def _publish(payload: dict[str, Any], root: Path, lora_dir: Path) -> dict[str, Any]:
    key = str(payload.get("target_key") or "")
    if not TARGET_KEY_RE.match(key) or key.endswith(".partial"):
        raise EngineError(f"invalid target_key {key!r}")
    ck = payload.get("checkpoint") or {}
    expected = str(ck.get("sha256") or "").lower()
    src = _under_root(ck.get("path"), root)
    if not all((src / n).is_file() for n in ADAPTER_FILES):
        raise EngineError("checkpoint is incomplete on this node")
    digest = sha256_file(src / "adapter_model.safetensors")
    if digest != expected:
        raise EngineError(f"checkpoint sha256 {digest} does not match the expected {expected}")
    target = lora_dir / key
    if target.exists():
        existing = target / "adapter_model.safetensors"
        if (all((target / n).is_file() for n in ADAPTER_FILES) and sha256_file(existing) == digest):
            return {"published": {"key": key, "sha256": digest}}
        raise EngineError(f"LoRA '{key}' already exists on this node with different weights")
    lora_dir.mkdir(parents=True, exist_ok=True)
    staging = target.with_name(target.name + ".partial")
    shutil.rmtree(staging, ignore_errors=True)
    staging.mkdir()
    try:
        for name in ADAPTER_FILES:
            shutil.copy2(src / name, staging / name)
        if sha256_file(staging / "adapter_model.safetensors") != digest:
            raise EngineError("the copied adapter does not match its checksum")
        staging.rename(target)
    except BaseException:
        shutil.rmtree(staging, ignore_errors=True)
        raise
    log.info("published checkpoint %s as LoRA '%s'", src, key)
    return {"published": {"key": key, "sha256": digest}}


def _delete(payload: dict[str, Any], root: Path) -> dict[str, Any]:
    items = payload.get("checkpoints")
    if not isinstance(items, list):
        raise EngineError("delete job has no checkpoints")
    checked = []
    for it in items:  # check ALL first
        if not isinstance(it, dict):
            raise EngineError("invalid checkpoint entry")
        checked.append((str(it.get("id")), _under_root(it.get("path"), root)))
    deleted: list[str] = []
    missing: list[str] = []
    for cid, path in checked:
        if path.is_dir():
            shutil.rmtree(path)
            deleted.append(cid)
        else:
            missing.append(cid)
    return {"deleted": deleted, "missing": missing}


def run(payload: dict[str, Any], ctx: Any, *, checkpoint_root: Path, lora_dir: Path) -> dict[str, Any]:
    op = payload.get("op")
    if op == "publish":
        return _publish(payload, checkpoint_root, lora_dir)
    if op == "delete":
        return _delete(payload, checkpoint_root)
    raise EngineError(f"unknown lora_files op {op!r}")
