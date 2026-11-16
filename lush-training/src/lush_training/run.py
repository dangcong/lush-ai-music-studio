"""A training run: immutable `run.json`, durable status, metrics and a STOP file.

    <TRAINING_DATA_ROOT>/runs/<run_id>/
      run.json          immutable: config, dataset + caption hashes, parent checkpoint SHA-256, method,
                        trainable / frozen split, code versions
      status.json       current state (atomic rewrite)
      metrics.jsonl     one line per optimizer step / validation
      events.jsonl      session starts, stops, checkpoints, rejected checkpoints, retention
      sessions/         one environment record per session (dependencies, GPU)
      checkpoints/      see checkpoint.py
      STOP              created to ask the trainer to stop safely
      trainer.pid       pid of the running trainer
      trainer.log       stdout/stderr of a detached trainer
"""
from __future__ import annotations

import os
import re
import subprocess
from datetime import datetime
from pathlib import Path
from typing import Any, Optional

from . import captions, checkpoint, dataset, model
from .acestep_probe import repo_commit
from .config import WORKSPACE, Project, load_yaml
from .util import (LushTrainingError, append_jsonl, atomic_write_bytes, atomic_write_json, now_iso, read_json,
                   read_jsonl, sha256_file)

SCHEMA = 1
NAME_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
OPTIMIZERS = ("adamw", "adamw8bit", "adafactor")
SCHEDULERS = ("constant", "cosine")
DEFAULTS: dict[str, Any] = {
    "trainable_patterns": [],
    "precision": {"weights": "fp32", "autocast": "bf16"},
    "optimizer": {"type": "adamw", "learning_rate": None, "weight_decay": 0.01},
    "scheduler": {"type": "cosine", "warmup_steps": 0, "min_lr_ratio": 0.1},
    "batch_size": 1, "gradient_accumulation": 1, "gradient_checkpointing": True, "max_grad_norm": 1.0,
    "cfg_dropout": 0.15, "seed": 42,
    "limits": {"max_steps": None, "max_epochs": None, "max_hours": None},
    "checkpoint": {"every_steps": 0, "every_minutes": 30, "keep_resumable": 2, "milestone_every_steps": 0},
    "validation": {"every_steps": 0, "seed": 1234, "early_stopping_patience": None},
    "preflight": {"min_free_disk_gb": 60, "min_free_vram_mib": None},
}


def _merge(defaults: dict[str, Any], given: dict[str, Any], where: str) -> dict[str, Any]:
    unknown = set(given) - set(defaults)
    if unknown:
        raise LushTrainingError(f"{where}: khóa không hợp lệ {sorted(unknown)}")
    out = dict(defaults)
    out.update(given)
    return out


def load_training_config(path: Path) -> dict[str, Any]:
    """Validate a training config. Anything not understood is an error, not a silent default."""
    raw = load_yaml(path)
    where = str(path)
    allowed = set(DEFAULTS) | {"run_name", "dataset", "model", "method"}
    if set(raw) - allowed:
        raise LushTrainingError(f"{where}: khóa không hợp lệ {sorted(set(raw) - allowed)}")
    cfg: dict[str, Any] = {}
    cfg["run_name"] = str(raw.get("run_name") or "")
    if not NAME_RE.match(cfg["run_name"]):
        raise LushTrainingError(f"{where}: `run_name` phải khớp {NAME_RE.pattern}")
    ds = raw.get("dataset") or {}
    for key in ("name", "version", "caption_version"):
        if not ds.get(key):
            raise LushTrainingError(f"{where}: thiếu dataset.{key}")
    cfg["dataset"] = {k: str(ds[k]) for k in ("name", "version", "caption_version")}
    md = raw.get("model") or {}
    if not md.get("checkpoint"):
        raise LushTrainingError(f"{where}: thiếu model.checkpoint (tên thư mục trong vùng pretrained)")
    cfg["model"] = {"checkpoint": str(md["checkpoint"]), "license": md.get("license"), "source": md.get("source")}
    cfg["method"] = raw.get("method")
    if cfg["method"] not in model.TRAINING_METHODS:
        raise LushTrainingError(f"{where}: `method` phải là một trong {sorted(model.TRAINING_METHODS)}; "
                                "workspace không tự chọn thay bạn")
    for key, default in DEFAULTS.items():
        value = raw.get(key, default)
        cfg[key] = _merge(default, value or {}, f"{where}: {key}") if isinstance(default, dict) else value
    opt, sch, lim, ck = cfg["optimizer"], cfg["scheduler"], cfg["limits"], cfg["checkpoint"]
    if opt["type"] not in OPTIMIZERS:
        raise LushTrainingError(f"{where}: optimizer.type phải là một trong {OPTIMIZERS}")
    lr = opt["learning_rate"]
    if isinstance(lr, bool) or not isinstance(lr, (int, float)) or not 0 < lr <= 1e-2:
        raise LushTrainingError(f"{where}: optimizer.learning_rate bắt buộc, trong (0, 0.01]")
    if sch["type"] not in SCHEDULERS:
        raise LushTrainingError(f"{where}: scheduler.type phải là một trong {SCHEDULERS}")
    if not isinstance(lim["max_steps"], int) or isinstance(lim["max_steps"], bool) or lim["max_steps"] < 1:
        raise LushTrainingError(f"{where}: limits.max_steps bắt buộc (số optimizer step; lịch learning rate dựa vào nó)")
    for key in ("max_epochs", "max_hours"):
        v = lim[key]
        if v is not None and (isinstance(v, bool) or not isinstance(v, (int, float)) or v <= 0):
            raise LushTrainingError(f"{where}: limits.{key} phải là số dương hoặc null")
    if sch["warmup_steps"] < 0 or sch["warmup_steps"] >= lim["max_steps"]:
        raise LushTrainingError(f"{where}: scheduler.warmup_steps phải nhỏ hơn limits.max_steps")
    for key in ("batch_size", "gradient_accumulation"):
        if not isinstance(cfg[key], int) or cfg[key] < 1:
            raise LushTrainingError(f"{where}: {key} phải là số nguyên >= 1")
    if ck["every_steps"] <= 0 and ck["every_minutes"] <= 0:
        raise LushTrainingError(f"{where}: cần checkpoint.every_steps hoặc checkpoint.every_minutes > 0")
    if ck["keep_resumable"] < 1:
        raise LushTrainingError(f"{where}: checkpoint.keep_resumable phải >= 1")
    if cfg["precision"]["weights"] != "fp32":
        raise LushTrainingError(f"{where}: precision.weights chỉ hỗ trợ fp32 (cập nhật nhỏ bị làm tròn mất trên bf16)")
    if cfg["precision"]["autocast"] not in ("bf16", "fp32"):
        raise LushTrainingError(f"{where}: precision.autocast phải là bf16 hoặc fp32")
    if cfg["method"] == "partial_decoder" and not cfg["trainable_patterns"]:
        raise LushTrainingError(f"{where}: partial_decoder cần trainable_patterns")
    if cfg["method"] != "partial_decoder" and cfg["trainable_patterns"]:
        raise LushTrainingError(f"{where}: trainable_patterns chỉ dùng với method partial_decoder")
    return cfg


def workspace_commit() -> Optional[str]:
    commit = repo_commit(WORKSPACE)
    if commit is None:
        return None
    try:
        dirty = subprocess.run(["git", "-C", str(WORKSPACE), "status", "--porcelain", "--", "."],
                               capture_output=True, text=True, timeout=30).stdout.strip()
    except (OSError, subprocess.TimeoutExpired):
        dirty = ""
    return commit + ("+dirty" if dirty else "")


def tensor_dir(project: Project, cfg: dict[str, Any], split: str) -> Path:
    ds = cfg["dataset"]
    return project.dir("tensors") / f"{ds['name']}-{ds['version']}-{ds['caption_version']}-{cfg['model']['checkpoint']}" / split


def create_run(project: Project, cfg: dict[str, Any], *, now: Optional[datetime] = None) -> Path:
    """Create the run directory and its immutable run.json. Hashes the parent checkpoint (minutes for GBs)."""
    ds = cfg["dataset"]
    vdir = dataset.version_dir(project, ds["name"], ds["version"])
    manifest, manifest_sha = dataset.load_manifest(vdir)
    lock = captions.load_lock(vdir, ds["caption_version"])
    model_dir = project.checkpoints_dir / cfg["model"]["checkpoint"]
    report = model.inspect_checkpoint(model_dir, with_hash=True)
    compat = report["compatibility"]
    if not compat["compatible"]:
        raise LushTrainingError(f"checkpoint {model_dir.name} không tương thích backend nào: {compat['reasons']}")
    plan = model.trainable_plan(report, cfg["method"], cfg["trainable_patterns"], model.tensor_counts(model_dir))
    run_id = f"{cfg['run_name']}-{(now or datetime.now()).strftime('%Y%m%d-%H%M%S')}"
    run_dir = project.dir("runs") / run_id
    if run_dir.exists():
        raise LushTrainingError(f"run đã tồn tại: {run_dir}")
    run_dir.mkdir(parents=True)
    atomic_write_json(run_dir / "run.json", {
        "schema": SCHEMA, "run_id": run_id, "created_at": now_iso(), "instrumental_only": True, "config": cfg,
        "dataset": {**ds, "manifest_sha256": manifest_sha, "captions_sha256": lock["captions_sha256"],
                    "segments": lock["segments"], "totals": manifest["totals"], "rights": manifest["rights"],
                    "vocal_screening": manifest["vocal_screening"]},
        "parent": {"name": report["name"], "weights_sha256": report["weights_sha256"],
                   "weights_bytes": report["weights_bytes"], "total_parameters": report["total_parameters"],
                   "components": report["components"], "config": report["config"],
                   "license": cfg["model"]["license"], "source": cfg["model"]["source"],
                   "warnings": compat["warnings"]},
        "backend": compat["backend"], "training": plan,
        "code": {"lush_training_commit": workspace_commit(), "acestep_commit": repo_commit(project.acestep_repo)
                 if project.acestep_repo else None},
    })
    write_status(run_dir, {"state": "created", "step": 0})
    return run_dir


def load_run(run_dir: Path) -> dict[str, Any]:
    path = Path(run_dir) / "run.json"
    if not path.is_file():
        raise LushTrainingError(f"không có run tại {run_dir}")
    return read_json(path)


def find_run(project: Project, run: str) -> Path:
    """A run directory from a run id or a path."""
    candidate = Path(run)
    if (candidate / "run.json").is_file():
        return candidate.resolve()
    candidate = project.dir("runs") / run
    if (candidate / "run.json").is_file():
        return candidate
    raise LushTrainingError(f"không tìm thấy run `{run}` trong {project.dir('runs')}")


def write_status(run_dir: Path, status: dict[str, Any]) -> None:
    atomic_write_json(Path(run_dir) / "status.json", {**status, "updated_at": now_iso()})


def read_status(run_dir: Path) -> dict[str, Any]:
    path = Path(run_dir) / "status.json"
    return read_json(path) if path.is_file() else {"state": "unknown"}


def event(run_dir: Path, kind: str, **data: Any) -> None:
    append_jsonl(Path(run_dir) / "events.jsonl", {"at": now_iso(), "event": kind, **data})


def request_stop(run_dir: Path, reason: str = "user") -> None:
    (Path(run_dir) / "STOP").write_text(f"{now_iso()} {reason}\n", encoding="utf-8")


def pid_alive(pid: int) -> bool:
    if pid <= 0:
        return False
    if os.name == "nt":
        import ctypes
        handle = ctypes.windll.kernel32.OpenProcess(0x1000, False, pid)  # PROCESS_QUERY_LIMITED_INFORMATION
        if not handle:
            return False
        code = ctypes.c_ulong()
        ctypes.windll.kernel32.GetExitCodeProcess(handle, ctypes.byref(code))
        ctypes.windll.kernel32.CloseHandle(handle)
        return code.value == 259  # STILL_ACTIVE
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return False
    except PermissionError:
        return True
    return True


def trainer_pid(run_dir: Path) -> Optional[int]:
    """pid of a trainer that is still alive for this run, else None."""
    path = Path(run_dir) / "trainer.pid"
    try:
        pid = int(path.read_text().strip())
    except (OSError, ValueError):
        return None
    return pid if pid_alive(pid) else None


def describe(run_dir: Path, tail: int = 5) -> dict[str, Any]:
    """Everything `status.py` shows. A state that claims to be running without a live process is reported as such."""
    info = load_run(run_dir)
    status = read_status(run_dir)
    pid = trainer_pid(run_dir)
    if status.get("state") == "running" and pid is None:
        status = {**status, "state": "interrupted",
                  "note": "status ghi đang chạy nhưng không còn tiến trình trainer (crash, tắt máy hoặc logout). "
                          "Phần sau checkpoint gần nhất đã mất; dùng resume."}
    latest, rejected = checkpoint.latest_valid(run_dir)
    metrics = read_jsonl(Path(run_dir) / "metrics.jsonl")
    steps = [m for m in metrics if m.get("kind") == "step"]
    return {"run_id": info["run_id"], "run_dir": str(run_dir), "method": info["training"]["method"],
            "trainable_parameters": info["training"]["trainable_parameters"],
            "total_parameters": info["training"]["total_parameters"],
            "max_steps": info["config"]["limits"]["max_steps"], "status": status, "trainer_pid": pid,
            "stop_requested": (Path(run_dir) / "STOP").exists(),
            "latest_valid_checkpoint": latest.name if latest else None, "rejected_checkpoints": rejected,
            "checkpoints": [checkpoint.step_dir(run_dir, s).name for s in checkpoint.list_steps(run_dir)],
            "recent_steps": steps[-tail:],
            "validations": [m for m in metrics if m.get("kind") == "validation"][-tail:],
            "events": read_jsonl(Path(run_dir) / "events.jsonl")[-tail:]}


def parent_sha_matches(project: Project, info: dict[str, Any]) -> bool:
    weights = project.checkpoints_dir / info["parent"]["name"] / model.WEIGHTS_FILE
    return weights.is_file() and sha256_file(weights) == info["parent"]["weights_sha256"]


def _hms(seconds: Optional[float]) -> str:
    if seconds is None:
        return "chưa đủ số đo"
    seconds = int(seconds)
    return f"{seconds // 3600} giờ {seconds % 3600 // 60:02d} phút"


def write_progress_report(run_dir: Path) -> Path:
    """Rewrite `PROGRESS.md`: where the run is, what will stop it and when. Every number comes from the
    run's own files; a value that has not been measured yet is shown as such, never estimated."""
    run_dir = Path(run_dir)
    info = load_run(run_dir)
    cfg = info["config"]
    lim, val_cfg = cfg["limits"], cfg["validation"]
    d = describe(run_dir, tail=10)
    s = d["status"]
    step, max_steps = int(s.get("step") or 0), lim["max_steps"]
    spe = s.get("steps_per_epoch")
    metrics = read_jsonl(run_dir / "metrics.jsonl")
    steps = [m for m in metrics if m.get("kind") == "step"]
    vals = [m for m in metrics if m.get("kind") == "validation"]
    bounded = max_steps < 10 ** 8
    lines = [f"# Tiến độ — run `{info['run_id']}`", "",
             f"Cập nhật: {now_iso()} (UTC). Trainer ghi lại file này khoảng mỗi phút và mỗi lần lưu checkpoint.", "",
             f"**Trạng thái: {s.get('state')}**" + (f" — {s.get('stop_reason')}" if s.get("stop_reason") else ""), ""]
    if s.get("note"):
        lines += [s["note"], ""]
    if s.get("error"):
        lines += [f"Lỗi: {s['error']}", ""]
    lines += ["## Đã làm được", ""]
    if bounded:
        lines.append(f"- Optimizer step: {step} / {max_steps} ({step / max_steps:.1%})")
    else:
        lines.append(f"- Optimizer step: {step} (run chỉ giới hạn theo thời gian, không có tổng số step)")
    if spe:
        total_epochs = f" / {lim['max_epochs']}" if lim["max_epochs"] else ""
        lines.append(f"- Epoch (lượt qua toàn bộ segment train): {step / spe:.2f}{total_epochs}; {spe} step mỗi epoch")
    lines.append(f"- Thời gian train cộng dồn: {_hms(s.get('train_seconds_total') or 0)}; số phiên: {s.get('session', 0)}")
    lines.append(f"- Tốc độ phiên này: {s.get('seconds_per_step', 'chưa có dữ liệu')} giây/step; "
                 f"VRAM đỉnh: {s.get('vram_peak_mib') or 'chưa có dữ liệu'} MiB")
    lines += ["", "## Khi nào dừng", "", "Dừng ở giới hạn đầu tiên đạt tới:", ""]
    eta = s.get("eta_seconds_to_max_steps")
    if bounded:
        lines.append(f"- Hết {max_steps} step" + (f" ({lim['max_epochs']} epoch)" if lim["max_epochs"] else "")
                     + f": còn khoảng {_hms(eta)} train" + ("" if eta is not None else " (cần 20 step đã đo trong phiên này)"))
    if lim["max_hours"]:
        left = lim["max_hours"] * 3600 - (s.get("train_seconds_total") or 0)
        lines.append(f"- Hết ngân sách {lim['max_hours']} giờ train cộng dồn: còn {_hms(max(0, left))}")
    if s.get("session_deadline_at"):
        when = datetime.fromtimestamp(s["session_deadline_at"]).strftime("%Y-%m-%d %H:%M")
        lines.append(f"- Hết giờ của phiên này: {when} (giờ máy); khi đó lưu checkpoint rồi dừng, chạy lại để train tiếp")
    if val_cfg["early_stopping_patience"]:
        lines.append(f"- Validation loss không cải thiện {val_cfg['early_stopping_patience']} lần liên tiếp "
                     f"(hiện tại: {s.get('validations_without_improvement', 0)} lần)")
    lines.append("- Có yêu cầu dừng (`train.py stop`): lưu checkpoint rồi dừng")
    lines.append("- Lỗi (hết VRAM, loss không hữu hạn): dừng ngay, không tự đổi cấu hình, không tự train lại")
    lines += ["", "## Loss", ""]
    if steps:
        recent = [m["loss"] for m in steps[-50:]]
        first = [m["loss"] for m in steps[:50]]
        lines.append(f"- Train loss: trung bình {len(recent)} step gần nhất {sum(recent) / len(recent):.4f}; "
                     f"{len(first)} step đầu {sum(first) / len(first):.4f}; step cuối {steps[-1]['loss']:.4f}")
        lines.append(f"- Learning rate hiện tại: {steps[-1].get('lr')}")
    else:
        lines.append("- Chưa có step nào.")
    if vals:
        best = min(vals, key=lambda m: m["validation_loss"])
        lines += ["", "| Step | Epoch | Validation loss | |", "|---|---|---|---|"]
        for m in vals[-12:]:
            mark = "thấp nhất" if m is best else ("cải thiện" if m.get("improved") else "")
            epoch = f"{m['step'] / spe:.1f}" if spe else ""
            lines.append(f"| {m['step']} | {epoch} | {m['validation_loss']:.4f} | {mark} |")
    elif val_cfg["every_steps"]:
        lines.append(f"- Validation loss: chưa có (tính mỗi {val_cfg['every_steps']} step).")
    lines += ["", "Loss giảm không có nghĩa nhạc hay hơn; validation loss tăng trở lại là dấu hiệu bắt đầu học thuộc.",
              "", "## Checkpoint", "",
              f"- Hợp lệ mới nhất: {d['latest_valid_checkpoint'] or 'chưa có'}",
              f"- Đang có: {', '.join(d['checkpoints']) or 'chưa có'}"]
    for r in d["rejected_checkpoints"]:
        lines.append(f"- HỎNG: {r['checkpoint']}: {r['problems']}")
    lines += ["", "## Sự kiện gần đây", ""]
    for e in d["events"]:
        extra = {k: v for k, v in e.items() if k not in ("at", "event") and v not in (None, [], {})}
        lines.append(f"- {e['at']} {e['event']} {extra if extra else ''}".rstrip())
    out = run_dir / "PROGRESS.md"
    atomic_write_bytes(out, ("\n".join(lines) + "\n").encode("utf-8"))
    return out
