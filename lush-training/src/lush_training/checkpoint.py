"""Resume checkpoints: written atomically, verified by manifest, never silently replaced.

Write order: `step-N.partial/` -> every file fsynced -> `MANIFEST.json` (sizes + SHA-256) written
last -> directory renamed to `step-N/` -> `LATEST` updated. A crash at any point leaves the previous
checkpoint untouched; a leftover `.partial` directory is removed at the next start.
"""
from __future__ import annotations

import os
import re
from pathlib import Path
from typing import Any, Callable, Optional

from .util import (LushTrainingError, atomic_write_bytes, atomic_write_json, file_manifest, fsync_dir, fsync_file,
                   now_iso, read_json, remove_tree, verify_manifest)

STEP_DIR = re.compile(r"^step-(\d{8})$")
MANIFEST = "MANIFEST.json"
PROGRESS = "progress.json"


def checkpoints_dir(run_dir: Path) -> Path:
    return Path(run_dir) / "checkpoints"


def step_dir(run_dir: Path, step: int) -> Path:
    return checkpoints_dir(run_dir) / f"step-{step:08d}"


def list_steps(run_dir: Path) -> list[int]:
    root = checkpoints_dir(run_dir)
    if not root.is_dir():
        return []
    return sorted(int(m.group(1)) for p in root.iterdir() if p.is_dir() and (m := STEP_DIR.match(p.name)))


def remove_partials(run_dir: Path) -> list[str]:
    root = checkpoints_dir(run_dir)
    removed = []
    for p in root.glob("*.partial") if root.is_dir() else []:
        remove_tree(p)
        removed.append(p.name)
    return removed


def verify(ckpt: Path) -> list[str]:
    """Problems with one checkpoint directory. Empty = intact and resumable."""
    mpath = Path(ckpt) / MANIFEST
    if not mpath.is_file():
        return ["thiếu MANIFEST.json (checkpoint ghi dở)"]
    try:
        manifest = read_json(mpath)
    except ValueError:
        return ["MANIFEST.json hỏng"]
    if manifest.get("kind") != "resumable":
        return [f"không phải checkpoint resume (kind = {manifest.get('kind')})"]
    return verify_manifest(ckpt, manifest["files"])


def save(run_dir: Path, step: int, write_state: Callable[[Path], None], progress: dict[str, Any]) -> Path:
    """Write one resumable checkpoint. `write_state(dir)` writes the backend's files into `dir`."""
    final = step_dir(run_dir, step)
    if final.exists():
        raise LushTrainingError(f"checkpoint {final.name} đã tồn tại; không ghi đè")
    partial = final.with_name(final.name + ".partial")
    remove_tree(partial)
    partial.mkdir(parents=True)
    write_state(partial)
    atomic_write_json(partial / PROGRESS, progress)
    for p in partial.rglob("*"):
        if p.is_file():
            fsync_file(p)
    atomic_write_json(partial / MANIFEST, {"kind": "resumable", "step": step, "written_at": now_iso(),
                                           "files": file_manifest(partial)})
    os.replace(partial, final)
    fsync_dir(final.parent)
    problems = verify(final)
    if problems:
        raise LushTrainingError(f"checkpoint {final.name} vừa ghi không qua kiểm tra: {problems}")
    atomic_write_bytes(Path(run_dir) / "LATEST", (final.name + "\n").encode())
    return final


def latest_valid(run_dir: Path) -> tuple[Optional[Path], list[dict[str, Any]]]:
    """Newest intact resumable checkpoint, plus every newer one that was rejected and why."""
    rejected = []
    for step in reversed(list_steps(run_dir)):
        ckpt = step_dir(run_dir, step)
        problems = verify(ckpt)
        if not problems:
            return ckpt, rejected
        if not any("không phải checkpoint resume" in p for p in problems):
            rejected.append({"checkpoint": ckpt.name, "problems": problems})
    return None, rejected


def apply_retention(run_dir: Path, keep_resumable: int, milestone_every_steps: int,
                    weight_files: tuple[str, ...]) -> list[str]:
    """The only place this workspace deletes training output, and only as configured in run.json.

    Keeps the newest `keep_resumable` intact checkpoints. An older checkpoint on a milestone step is
    reduced to its weight files (kind = "milestone": can be evaluated, cannot be resumed from);
    other older checkpoints are removed. Damaged checkpoints are left alone for inspection.
    """
    actions = []
    intact = [s for s in list_steps(run_dir) if not verify(step_dir(run_dir, s))]
    for step in intact[:-keep_resumable] if keep_resumable > 0 else []:
        ckpt = step_dir(run_dir, step)
        if milestone_every_steps > 0 and step % milestone_every_steps == 0:
            manifest = read_json(ckpt / MANIFEST)
            keep = {n: m for n, m in manifest["files"].items() if n in weight_files or n == PROGRESS}
            for name in manifest["files"]:
                if name not in keep:
                    (ckpt / name).unlink()
            atomic_write_json(ckpt / MANIFEST, {**manifest, "kind": "milestone", "files": keep})
            actions.append(f"{ckpt.name}: giữ làm mốc (chỉ weights)")
        else:
            remove_tree(ckpt)
            actions.append(f"{ckpt.name}: xóa theo retention")
    return actions
