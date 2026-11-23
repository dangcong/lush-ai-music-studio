"""Export a locked dataset version for Lush Studio's Train Style (`train_lora`) worker.

The worker reads `<DATA_DIR>/datasets/<name>/<version>/dataset.json` + `audio/`
(`workers/music-worker/src/lush_worker/training.py`). Only the train split is exported as samples;
validation segments stay in this workspace for evaluation and are listed in `manifest.json`.
An exported version is written once (staged in `.partial`, then renamed) and made read-only.
"""
from __future__ import annotations

import os
import re
import shutil
import stat
from pathlib import Path
from typing import Any

from . import captions
from .dataset import load_manifest
from .util import LushTrainingError, atomic_write_json, now_iso, read_json, remove_tree, sha256_file

# Same pattern the Studio worker accepts (training.py DATASET_KEY_RE).
# Same character rules as the Studio worker (training.py DATASET_KEY_RE), but `<name>/<version>` is required:
# the worker only lists `*/*/dataset.json`, so a one-part key would never show up.
STUDIO_KEY_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}/[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
STAGING = ".studio-export-staging"  # beside the datasets root, never inside it
SCHEMA = 1


def _make_read_only(root: Path) -> None:
    for path in sorted(root.rglob("*"), reverse=True) + [root]:
        mode = path.stat().st_mode
        os.chmod(path, mode & ~(stat.S_IWUSR | stat.S_IWGRP | stat.S_IWOTH))


def export_studio(vdir: Path, caption_version: str, studio_root: Path, key: str) -> dict[str, Any]:
    if not STUDIO_KEY_RE.match(key or ""):
        raise LushTrainingError(f"dataset key không hợp lệ cho Studio: {key!r} (dạng `<tên>/<version>`)")
    vdir = Path(vdir)
    lock = captions.load_lock(vdir, caption_version)
    manifest, manifest_sha = load_manifest(vdir, verify_files=True)
    captions._refuse_excluded(vdir, manifest)
    target = Path(studio_root) / key
    if target.exists():
        raise LushTrainingError(f"dataset Studio đã tồn tại và bất biến: {target}. Dùng version mới.")
    engine = read_json(vdir / "captions" / caption_version / "engine.train.json")
    segments = {Path(s["file"]).name: s for s in manifest["segments"]}

    # Staged outside the datasets root: the worker lists `<root>/*/*/dataset.json`, so a half-written
    # export inside the root could be reported to the website.
    partial = Path(studio_root).resolve().parent / STAGING / key.replace("/", "--")
    remove_tree(partial)
    try:
        (partial / "audio").mkdir(parents=True)
        for sample in engine["samples"]:
            seg = segments[sample["audio_path"]]
            if seg["split"] != "train":
                raise LushTrainingError(f"mẫu {seg['segment_id']} không thuộc split train")
            shutil.copyfile(vdir / seg["file"], partial / "audio" / sample["audio_path"])
        dataset_json = {**engine, "metadata": {**engine["metadata"], "name": key.replace("/", "-")}}
        atomic_write_json(partial / "dataset.json", dataset_json)

        per_track: dict[str, int] = {}
        for s in manifest["segments"]:
            per_track[s["track_id"]] = per_track.get(s["track_id"], 0) + 1
        split_tracks = [t["track_id"] for t in manifest["tracks"] if per_track.get(t["track_id"], 0) > 1]
        kept_seconds: dict[str, float] = {}
        for seg in manifest["segments"]:
            kept_seconds[seg["track_id"]] = kept_seconds.get(seg["track_id"], 0.0) + float(seg["duration"])
        truncated = [{"track_id": t["track_id"], "duration": t["metrics"]["duration"],
                      "dropped_seconds": round(t["metrics"]["duration"] - kept_seconds.get(t["track_id"], 0.0), 1)}
                     for t in manifest["tracks"]
                     if t["metrics"]["duration"] - kept_seconds.get(t["track_id"], 0.0) > 1.0]
        validation = [s["segment_id"] for s in manifest["segments"] if s["split"] == "validation"]
        info = {
            "schema": SCHEMA, "key": key, "exported_at": now_iso(),
            "source": {"dataset": manifest["dataset"], "manifest_sha256": manifest_sha,
                       "version_dir": str(vdir)},
            "caption_version": caption_version, "captions_content_sha256": lock["content_sha256"],
            "sources": [{"track_id": t["track_id"], "sha256": t["sha256"], "source_path": t["source_path"],
                         "segments": t["segments"]} for t in manifest["tracks"] if t["split"] == "train"],
            "samples": [{"audio_path": s["audio_path"], "segment_id": segments[s["audio_path"]]["segment_id"],
                         "start": segments[s["audio_path"]]["start"], "end": segments[s["audio_path"]]["end"],
                         "sha256": sha256_file(partial / "audio" / s["audio_path"])} for s in engine["samples"]],
            "validation_segments": validation,
            "split_tracks": split_tracks,
            "truncated_tracks": truncated,
            "excluded": manifest["excluded"],
        }
        atomic_write_json(partial / "manifest.json", info)
        target.parent.mkdir(parents=True, exist_ok=True)
        os.replace(partial, target)
    except BaseException:
        remove_tree(partial)
        raise
    _make_read_only(target)
    return {"key": key, "dir": str(target), "train_samples": len(engine["samples"]),
            "split_tracks": split_tracks, "truncated_tracks": truncated, "excluded_validation": validation}
