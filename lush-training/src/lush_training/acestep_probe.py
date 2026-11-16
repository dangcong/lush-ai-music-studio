"""Check an ACE-Step checkout without importing it (no torch needed).

The trainer backend imports a handful of upstream functions. This probe confirms, by parsing the
source, that the checkout is at the pinned commit and still defines them. A different commit is a
hard stop: upstream training code changes between commits.
"""
from __future__ import annotations

import ast
import subprocess
from pathlib import Path
from typing import Any, Optional

PINNED_COMMIT = "ca1e85fe9430179831e6bc6be790c332190a3866"

# file -> symbols the backend uses, as read from the pinned commit.
REQUIRED_SYMBOLS = {
    "acestep/training_v2/model_loader.py": ["load_decoder_for_training", "read_model_config", "_resolve_model_dir"],
    "acestep/training_v2/timestep_sampling.py": ["sample_timesteps", "apply_cfg_dropout"],
    "acestep/training_v2/trainer_helpers.py": ["configure_memory_features"],
    "acestep/training/data_module.py": ["PreprocessedTensorDataset", "collate_preprocessed_batch"],
    "acestep/training/path_safety.py": ["set_safe_root"],
    "train.py": [],
}


def repo_commit(repo: Path) -> Optional[str]:
    try:
        proc = subprocess.run(["git", "-C", str(repo), "rev-parse", "HEAD"], capture_output=True, text=True, timeout=15)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return proc.stdout.strip() if proc.returncode == 0 else None


def repo_dirty(repo: Path) -> Optional[bool]:
    try:
        proc = subprocess.run(["git", "-C", str(repo), "status", "--porcelain", "--untracked-files=no"],
                              capture_output=True, text=True, timeout=30)
    except (OSError, subprocess.TimeoutExpired):
        return None
    return bool(proc.stdout.strip()) if proc.returncode == 0 else None


def defined_names(source: str) -> set[str]:
    tree = ast.parse(source)
    return {n.name for n in tree.body if isinstance(n, (ast.FunctionDef, ast.AsyncFunctionDef, ast.ClassDef))}


def probe_repo(repo: Path, expected_commit: Optional[str] = None) -> dict[str, Any]:
    repo = Path(repo)
    expected = expected_commit or PINNED_COMMIT
    problems: list[str] = []
    if not repo.is_dir():
        return {"ok": False, "repo": str(repo), "commit": None, "problems": [f"không có thư mục {repo}"]}
    commit = repo_commit(repo)
    if commit is None:
        problems.append("không đọc được commit (không phải git checkout?)")
    elif not commit.startswith(expected) and not expected.startswith(commit):
        problems.append(f"commit {commit[:12]} khác bản đã ghim {expected[:12]}: backend chỉ được kiểm với bản ghim")
    if repo_dirty(repo):
        problems.append("checkout có file đã sửa: workspace này không sửa ACE-Step, hãy kiểm tra lại")
    for rel, symbols in REQUIRED_SYMBOLS.items():
        path = repo / rel
        if not path.is_file():
            problems.append(f"thiếu {rel}")
            continue
        if symbols:
            try:
                names = defined_names(path.read_text(encoding="utf-8"))
            except SyntaxError as e:
                problems.append(f"{rel}: không phân tích được ({e})")
                continue
            for s in symbols:
                if s not in names:
                    problems.append(f"{rel}: không còn định nghĩa `{s}`")
    return {"ok": not problems, "repo": str(repo), "commit": commit, "expected_commit": expected, "problems": problems}
