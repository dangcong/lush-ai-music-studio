"""Hashing, atomic writes and small JSON helpers shared by every module."""
from __future__ import annotations

import hashlib
import json
import os
import shutil
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable


class LushTrainingError(Exception):
    """A problem the user has to look at. The message says what and where."""


def now_iso() -> str:
    return datetime.now(timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ")


def sha256_file(path: Path, chunk: int = 1 << 20) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        while block := f.read(chunk):
            digest.update(block)
    return digest.hexdigest()


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def canonical_json(obj: Any) -> bytes:
    """Stable bytes for hashing a JSON value."""
    return json.dumps(obj, sort_keys=True, ensure_ascii=False, separators=(",", ":")).encode("utf-8")


def read_json(path: Path) -> Any:
    return json.loads(Path(path).read_text(encoding="utf-8"))


def fsync_dir(path: Path) -> None:
    """Flush a directory entry. Windows cannot open a directory for fsync, so this is a no-op there."""
    if os.name == "nt":
        return
    fd = os.open(str(path), os.O_RDONLY)
    try:
        os.fsync(fd)
    finally:
        os.close(fd)


def replace_with_retry(src: Path, dst: Path, seconds: float = 5.0) -> None:
    """`os.replace`, retried while Windows refuses because another process has the target open
    (for example `status.py` reading `status.json` at that instant). POSIX never needs the retry."""
    deadline = time.monotonic() + seconds
    while True:
        try:
            os.replace(src, dst)
            return
        except PermissionError:
            if os.name != "nt" or time.monotonic() >= deadline:
                raise
            time.sleep(0.02)


def atomic_write_bytes(path: Path, data: bytes) -> None:
    """Write through a temporary file in the same directory, fsync, then rename over the target."""
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    tmp = path.with_name(f".{path.name}.tmp-{os.getpid()}")
    with open(tmp, "wb") as f:
        f.write(data)
        f.flush()
        os.fsync(f.fileno())
    replace_with_retry(tmp, path)
    fsync_dir(path.parent)


def atomic_write_json(path: Path, obj: Any) -> None:
    atomic_write_bytes(path, (json.dumps(obj, indent=2, ensure_ascii=False) + "\n").encode("utf-8"))


def append_jsonl(path: Path, obj: Any) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "a", encoding="utf-8") as f:
        f.write(json.dumps(obj, ensure_ascii=False) + "\n")
        f.flush()


def read_jsonl(path: Path) -> list[Any]:
    if not Path(path).is_file():
        return []
    rows = []
    for line in Path(path).read_text(encoding="utf-8").splitlines():
        line = line.strip()
        if line:
            try:
                rows.append(json.loads(line))
            except ValueError:
                continue  # a line cut short by a crash: the rest of the file is still good
    return rows


def fsync_file(path: Path) -> None:
    with open(path, "rb+") as f:
        os.fsync(f.fileno())


def file_manifest(root: Path, names: Iterable[str] | None = None) -> dict[str, dict[str, Any]]:
    """`{relative posix path: {bytes, sha256}}` for every file under root (or only `names`)."""
    root = Path(root)
    if names is None:
        paths = sorted(p for p in root.rglob("*") if p.is_file())
    else:
        paths = [root / n for n in names]
    return {p.relative_to(root).as_posix(): {"bytes": p.stat().st_size, "sha256": sha256_file(p)} for p in paths}


def verify_manifest(root: Path, files: dict[str, dict[str, Any]]) -> list[str]:
    """Problems found when comparing files on disk with a manifest. Empty list = intact."""
    problems = []
    for rel, meta in files.items():
        p = Path(root) / rel
        if not p.is_file():
            problems.append(f"missing: {rel}")
        elif p.stat().st_size != meta["bytes"]:
            problems.append(f"size differs: {rel}")
        elif sha256_file(p) != meta["sha256"]:
            problems.append(f"sha256 differs: {rel}")
    return problems


def remove_tree(path: Path) -> None:
    shutil.rmtree(path, ignore_errors=True)


def monotonic() -> float:
    return time.monotonic()
