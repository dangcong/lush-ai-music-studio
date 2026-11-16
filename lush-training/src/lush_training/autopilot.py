"""Autopilot: one job file -> dataset -> captions -> training -> selected checkpoint -> inference package.

The user writes ONE job file that carries a standing authorization for that job only: rights are
confirmed, captions may be approved automatically, the GPU may be used until the stop condition.
Everything the pipeline then decides on its own is recorded as such, so nobody later mistakes it for
a human decision:

- captions come from the descriptions in the job file (whole library or per sub-folder). Nobody and
  nothing listened to each segment;
- the checkpoint is selected by validation loss, which is not evidence of better music;
- no vocal screening is done.

Re-running the same job continues where it stopped (each stage is idempotent). Installing the engine,
downloading checkpoints and copying the package to another machine are NOT part of autopilot.
"""
from __future__ import annotations

import hashlib
import math
import subprocess
import sys
import time
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any, Callable, Optional

import yaml

from . import captions, checkpoint, dataset, export, run as runs, session
from .config import WORKSPACE, Project, load_yaml
from .util import LushTrainingError, atomic_write_json, now_iso, read_json, read_jsonl, verify_manifest

DEFAULT_BACKEND = "lush_training.acestep_backend:AceStepDecoderBackend"
CAPTION_VERSION = "auto"
EXPORT_RESERVE_SECONDS = 600
STOP_KEYS = {"epochs", "total_train_hours", "patience_epochs", "hours", "at"}
AUTHORIZATION_KEYS = ("rights_confirmed", "auto_approve_captions", "use_gpu")
TRAINING_KEYS = ("optimizer", "scheduler", "batch_size", "gradient_accumulation", "gradient_checkpointing",
                 "max_grad_norm", "cfg_dropout", "seed", "preflight", "trainable_patterns", "precision")


def load_job(path: Path) -> dict[str, Any]:
    job = load_yaml(path)
    where = str(path)
    allowed = {"name", "authorized_by", "authorization", "music_dir", "rights", "captions", "segment", "split",
               "model", "method", "training", "stop", "export_name"}
    if set(job) - allowed:
        raise LushTrainingError(f"{where}: khóa không hợp lệ {sorted(set(job) - allowed)}")
    if not dataset.VERSION_RE.match(str(job.get("name") or "")):
        raise LushTrainingError(f"{where}: `name` không hợp lệ")
    if not str(job.get("authorized_by") or "").strip():
        raise LushTrainingError(f"{where}: thiếu `authorized_by` (tên người cho phép job này chạy tự động)")
    auth = job.get("authorization") or {}
    missing = [k for k in AUTHORIZATION_KEYS if auth.get(k) is not True]
    if missing:
        raise LushTrainingError(f"{where}: autopilot cần người dùng tự đặt authorization.{', '.join(missing)}: true. "
                                "Claude Code không tự điền các dòng này.")
    if not job.get("music_dir"):
        raise LushTrainingError(f"{where}: thiếu `music_dir`")
    caps = job.get("captions") or {}
    if not str(caps.get("default") or "").strip() and not caps.get("by_folder"):
        raise LushTrainingError(f"{where}: cần captions.default hoặc captions.by_folder (mô tả nhạc bằng lời của bạn)")
    for text in [caps.get("default") or ""] + list((caps.get("by_folder") or {}).values()):
        stripped = captions.NEGATED_VOCAL.sub(" ", str(text))
        if captions.VOCAL_TERMS.search(stripped):
            raise LushTrainingError(f"{where}: mô tả nhắc tới vocal: {text!r}. Workspace chỉ train instrumental.")
    if not (job.get("model") or {}).get("checkpoint"):
        raise LushTrainingError(f"{where}: thiếu model.checkpoint")
    if not job.get("method"):
        raise LushTrainingError(f"{where}: thiếu `method`")
    stop = job.get("stop") or {}
    if set(stop) - STOP_KEYS:
        raise LushTrainingError(f"{where}: khóa không hợp lệ trong stop: {sorted(set(stop) - STOP_KEYS)}")
    if not stop.get("epochs") and not stop.get("total_train_hours"):
        raise LushTrainingError(f"{where}: `stop` cần epochs hoặc total_train_hours để job có điểm kết thúc hẳn. "
                                "Chỉ có hours/at thì mỗi lần chạy lại job lại train tiếp mãi.")
    training = job.get("training") or {}
    if set(training) - set(TRAINING_KEYS):
        raise LushTrainingError(f"{where}: khóa không hợp lệ trong training: {sorted(set(training) - set(TRAINING_KEYS))}")
    if (training.get("optimizer") or {}).get("learning_rate") is None:
        raise LushTrainingError(f"{where}: thiếu training.optimizer.learning_rate")
    return job


def deadline_of(stop: dict[str, Any], started: float) -> Optional[float]:
    candidates = []
    if stop.get("hours"):
        candidates.append(started + float(stop["hours"]) * 3600)
    if stop.get("at"):
        hour, minute = (int(x) for x in str(stop["at"]).split(":"))
        base = datetime.fromtimestamp(started)
        target = base.replace(hour=hour, minute=minute, second=0, microsecond=0)
        if target <= base:
            target += timedelta(days=1)
        candidates.append(target.timestamp())
    return min(candidates) if candidates else None


class Autopilot:
    def __init__(self, project: Project, job_path: Path, *, project_path: Optional[Path] = None,
                 backend: str = DEFAULT_BACKEND, preprocess: Optional[Callable[[Path], int]] = None, clock: Callable[[], float] = time.time,
                 log: Callable[[str], None] = print):
        self.project = project
        self.job = load_job(job_path)
        self.backend = backend
        self.project_args = ["--project", str(project_path)] if project_path else []
        self._preprocess = preprocess
        self.clock = clock
        self.log = log
        self.dir = project.data_root / "autopilot" / self.job["name"]
        self.dir.mkdir(parents=True, exist_ok=True)
        self.state_path = self.dir / "state.json"
        self.state: dict[str, Any] = read_json(self.state_path) if self.state_path.is_file() else {"stages": {}}

    # ---------- bookkeeping ----------
    def _done(self, stage: str, **data: Any) -> None:
        self.state["stages"][stage] = {"done_at": now_iso(), **data}
        atomic_write_json(self.state_path, self.state)
        self.log(f"[autopilot] {stage}: xong {data if data else ''}")

    def _who(self) -> str:
        return f"{self.job['authorized_by']} (autopilot, ủy quyền trong job file)"

    # ---------- stages ----------
    def stage_dataset(self) -> Path:
        job = self.job
        cfg_path = self.dir / "dataset.yaml"
        cfg_path.write_text(yaml.safe_dump({
            "name": job["name"], "source_dirs": [str(job["music_dir"])],
            "rights": {**(job.get("rights") or {}), "confirmed": True, "confirmed_by": job["authorized_by"]},
            "segment": job.get("segment") or {}, "split": {"test": 0.0, **(job.get("split") or {})},
        }, allow_unicode=True, sort_keys=False), encoding="utf-8")
        cfg = dataset.load_dataset_config(cfg_path)
        scanned = dataset.scan(cfg, self.project)
        # Same folder content + same settings -> same version name, so a re-run reuses the dataset.
        digest = hashlib.sha256((scanned["config_sha256"] + "|".join(sorted(
            t["sha256"] for t in scanned["tracks"] if not t["exclude_reason"]))).encode()).hexdigest()
        version = f"auto-{digest[:10]}"
        vdir = dataset.version_dir(self.project, job["name"], version)
        if not vdir.exists():
            dataset.build(cfg, self.project, version)
        manifest, _ = dataset.load_manifest(vdir)
        previous = self.state["stages"].get("dataset", {}).get("version")
        if previous and previous != version and "run" in self.state["stages"]:
            raise LushTrainingError(
                f"kho nhạc hoặc cấu hình cắt segment đã đổi sau khi job này bắt đầu train (dataset {previous} -> {version}). "
                "Run đang có gắn với dataset cũ: đặt `name` mới trong job file để train trên kho nhạc mới.")
        self._done("dataset", version=version, files_found=scanned["files_found"],
                   excluded=len(manifest["excluded"]), totals=manifest["totals"], vocal_screening="none")
        return vdir

    def _description(self, rel: str) -> tuple[str, str]:
        caps = self.job["captions"]
        folders = {str(k).replace("\\", "/").strip("/") + "/": str(v) for k, v in (caps.get("by_folder") or {}).items()}
        matches = [prefix for prefix in folders if rel.startswith(prefix)]
        if matches:
            best = max(matches, key=len)   # the most specific folder wins
            return folders[best], best
        if not str(caps.get("default") or "").strip():
            raise LushTrainingError(f"không có mô tả cho `{rel}`: thêm captions.default hoặc thư mục của nó vào captions.by_folder")
        return str(caps["default"]), "(mặc định)"

    def stage_captions(self, vdir: Path) -> None:
        lock_dir = vdir / "captions" / CAPTION_VERSION
        job_caps_sha = hashlib.sha256(yaml.safe_dump(self.job["captions"], sort_keys=True).encode()).hexdigest()
        if lock_dir.exists():
            if self.state["stages"].get("captions", {}).get("job_captions_sha256") != job_caps_sha:
                raise LushTrainingError("mô tả trong job file đã đổi sau khi caption được khóa. Caption đã khóa là bất biến: "
                                        "dùng `name` mới cho job, hoặc giữ nguyên mô tả cũ.")
            captions.load_lock(vdir, CAPTION_VERSION)
            return
        manifest, _ = dataset.load_manifest(vdir)
        music = Path(self.job["music_dir"]).expanduser().resolve()
        rel_of = {}
        for t in manifest["tracks"]:
            try:
                rel_of[t["track_id"]] = Path(t["source_path"]).resolve().relative_to(music).as_posix()
            except ValueError:
                rel_of[t["track_id"]] = Path(t["source_path"]).name
        captions.init_work(vdir)
        used: dict[str, int] = {}
        for seg in manifest["segments"]:
            if seg["split"] not in captions.TRAINING_SPLITS:
                continue
            text, source = self._description(rel_of[seg["track_id"]])
            used[source] = used.get(source, 0) + 1
            cap = read_json(captions.work_path(vdir, seg["segment_id"]))
            if cap["status"] == "approved":
                continue
            captions.set_fields(vdir, seg["segment_id"], by=self._who(), human=True, caption_text=text, status="approved",
                                note="autopilot: mô tả theo thư mục từ job file; chưa ai nghe từng segment")
        info = captions.lock(vdir, CAPTION_VERSION)
        self._done("captions", caption_version=CAPTION_VERSION, segments=info["segments"], descriptions_used=used,
                   job_captions_sha256=job_caps_sha,
                   basis="mô tả do người dùng ghi trong job file, áp theo thư mục; không có bước nghe")

    def stage_run(self, vdir: Path) -> Path:
        existing = self.state["stages"].get("run", {}).get("run_dir")
        if existing and (Path(existing) / "run.json").is_file():
            return Path(existing)
        job = self.job
        manifest, _ = dataset.load_manifest(vdir)
        training = dict(job.get("training") or {})
        per_step = int(training.get("batch_size", 1)) * int(training.get("gradient_accumulation", 1))
        steps_per_epoch = max(1, math.ceil(manifest["totals"]["train"]["segments"] / per_step))
        epochs = job["stop"].get("epochs")
        scheduler = dict(training.get("scheduler") or {})
        if epochs:
            max_steps = int(epochs) * steps_per_epoch
        else:
            # Time-limited only: there is no known end for a decaying schedule, so the rate stays constant.
            max_steps = 10 ** 9
            scheduler["type"] = "constant"
        scheduler["warmup_steps"] = min(int(scheduler.get("warmup_steps", 0)), max(0, max_steps - 1))
        has_val = manifest["totals"]["validation"]["segments"] > 0
        cfg = {**{k: v for k, v in training.items() if k != "scheduler"}, "scheduler": scheduler,
               "run_name": job["name"],
               "dataset": {"name": job["name"], "version": vdir.name, "caption_version": CAPTION_VERSION},
               "model": job["model"], "method": job["method"],
               "limits": {"max_steps": max_steps, "max_epochs": epochs,
                          "max_hours": job["stop"].get("total_train_hours")},
               # One checkpoint, one validation and one kept milestone per pass over the training set.
               "checkpoint": {"every_steps": steps_per_epoch, "every_minutes": 30, "keep_resumable": 2,
                              "milestone_every_steps": steps_per_epoch},
               "validation": {"every_steps": steps_per_epoch if has_val else 0, "seed": 1234,
                              "early_stopping_patience": job["stop"].get("patience_epochs") if has_val else None}}
        cfg_path = self.dir / "training.yaml"
        cfg_path.write_text(yaml.safe_dump(cfg, allow_unicode=True, sort_keys=False), encoding="utf-8")
        run_dir = runs.create_run(self.project, runs.load_training_config(cfg_path))
        self._done("run", run_dir=str(run_dir), steps_per_epoch=steps_per_epoch, max_steps=max_steps, epochs=epochs)
        return run_dir

    def stage_preprocess(self, run_dir: Path) -> None:
        info = runs.load_run(run_dir)
        need = info["dataset"]["segments"]
        have = {s: len(list(runs.tensor_dir(self.project, info["config"], s).glob("*.pt"))) for s in need}
        if self._preprocess is None and all(have[s] == need[s] for s in need):
            return
        started = self.clock()
        if self._preprocess is not None:
            code = self._preprocess(run_dir)
        else:
            code = subprocess.run([sys.executable, "-m", "lush_training.cli", "train", "preprocess", "--run", str(run_dir),
                                   "--confirm-gpu", *self.project_args], env=_env(), cwd=str(self.project.data_root)).returncode
        if code != 0:
            raise LushTrainingError(f"preprocess thất bại (exit {code}); xem preprocess.log trong thư mục tensors")
        self._done("preprocess", seconds=round(self.clock() - started, 1))

    def stage_train(self, run_dir: Path, deadline: Optional[float]) -> int:
        status = runs.read_status(run_dir)
        if status.get("state") == "completed":
            return session.EXIT_COMPLETED
        if status.get("state") == "failed":
            raise LushTrainingError(f"run đã thất bại ({status.get('stop_reason')}: {status.get('error')}). Autopilot không tự "
                                    "đổi cấu hình hay train lại từ đầu; xem `status.py`.")
        cmd = ["-m", "lush_training.cli", "train", "_worker", "--run", str(run_dir), "--backend", self.backend,
               *self.project_args]
        if deadline is not None:
            remaining = deadline - EXPORT_RESERVE_SECONDS - self.clock()
            if remaining <= 0:
                self.log("[autopilot] hết thời gian trước khi tới bước train")
                return session.EXIT_TIME_LIMIT
            cmd += ["--session-hours", f"{remaining / 3600:.4f}"]
        python = str(self.project.acestep_python) if self.backend == DEFAULT_BACKEND and self.project.acestep_python else sys.executable
        with open(run_dir / "trainer.log", "ab") as log:
            code = subprocess.run([python, *cmd], env=_env(), cwd=str(self.project.data_root), stdin=subprocess.DEVNULL,
                                  stdout=log, stderr=subprocess.STDOUT).returncode
        status = runs.read_status(run_dir)
        self._done("train", exit_code=code, state=status.get("state"), stop_reason=status.get("stop_reason"),
                   step=status.get("step"), train_seconds=status.get("train_seconds_total"),
                   vram_peak_mib=status.get("vram_peak_mib"))
        if code not in (session.EXIT_COMPLETED, session.EXIT_TIME_LIMIT, session.EXIT_STOP_REQUESTED):
            raise LushTrainingError(f"trainer dừng vì lỗi (exit {code}, {status.get('stop_reason')}: {status.get('error')}). "
                                    "Checkpoint tốt gần nhất còn nguyên. Autopilot không tự đổi cấu hình.")
        return code

    def select_checkpoint(self, run_dir: Path) -> dict[str, Any]:
        """Lowest validation loss among checkpoints that still have their weights; else the newest one."""
        available = []
        for step in checkpoint.list_steps(run_dir):
            manifest = read_json(checkpoint.step_dir(run_dir, step) / checkpoint.MANIFEST)
            if export.DECODER_FILE in manifest["files"] and not verify_manifest(checkpoint.step_dir(run_dir, step),
                                                                                manifest["files"]):
                available.append(step)
        if not available:
            raise LushTrainingError("chưa có checkpoint nào để xuất (run dừng trước checkpoint đầu tiên)")
        val = {m["step"]: m["validation_loss"] for m in read_jsonl(run_dir / "metrics.jsonl") if m.get("kind") == "validation"}
        scored = [(val[s], s) for s in available if s in val]
        if scored:
            loss, step = min(scored)
            return {"step": step, "basis": "validation loss thấp nhất", "validation_loss": loss,
                    "candidates": {str(s): val.get(s) for s in available}}
        return {"step": available[-1], "basis": "checkpoint mới nhất (không có validation loss)", "validation_loss": None,
                "candidates": {str(s): None for s in available}}

    def stage_export(self, run_dir: Path) -> Path:
        choice = self.select_checkpoint(run_dir)
        base = self.job.get("export_name") or self.job["name"]
        name = f"{base}-step{choice['step']}"
        target = self.project.dir("exports") / name
        if not target.exists():
            export.export_package(self.project, run_dir, checkpoint.step_dir(run_dir, choice["step"]).name, name)
        problems = export.verify_package(target)
        if problems:
            raise LushTrainingError(f"gói {name} không qua kiểm tra: {problems}")
        self._done("export", package=str(target), selected=choice)
        return target

    # ---------- driver ----------
    def run(self) -> dict[str, Any]:
        started = self.clock()
        deadline = deadline_of(self.job["stop"], started)
        self.state.setdefault("authorized_by", self.job["authorized_by"])
        self.state["last_started_at"] = now_iso()
        self.project.ensure_layout()
        vdir = self.stage_dataset()
        self.stage_captions(vdir)
        run_dir = self.stage_run(vdir)
        self.stage_preprocess(run_dir)
        code = self.stage_train(run_dir, deadline)
        package = self.stage_export(run_dir)
        finished = code == session.EXIT_COMPLETED
        self.state["result"] = {"finished": finished, "package": str(package), "run_dir": str(run_dir),
                                "seconds": round(self.clock() - started, 1)}
        atomic_write_json(self.state_path, self.state)
        self.write_report()
        return self.state["result"]

    def write_report(self) -> Path:
        st, res = self.state["stages"], self.state.get("result", {})
        ds, cp, tr, ex = st.get("dataset", {}), st.get("captions", {}), st.get("train", {}), st.get("export", {})
        sel = ex.get("selected", {})
        lines = [f"# Autopilot — job `{self.job['name']}`", "",
                 f"Cập nhật {now_iso()}. Ủy quyền bởi: {self.job['authorized_by']} (ghi trong job file).", "",
                 "## Kết quả", "",
                 f"- Trạng thái: {'ĐÃ XONG theo giới hạn epochs' if res.get('finished') else 'DỪNG THEO GIỜ, chưa hết số epoch; chạy lại cùng job để train tiếp'}.",
                 f"- Gói để bạn test: `{res.get('package')}`",
                 f"- Run: `{res.get('run_dir')}`", "",
                 "## Dataset", "",
                 f"- Version `{ds.get('version')}`; {ds.get('files_found')} file tìm thấy, {ds.get('excluded')} bị loại.",
                 f"- Theo tập: {ds.get('totals')}",
                 "- Vocal: KHÔNG có bước sàng lọc nào. Kho nhạc được coi là instrumental theo xác nhận của người ủy quyền.", "",
                 "## Caption", "",
                 f"- {cp.get('basis')}. Số segment theo mô tả: {cp.get('descriptions_used')}",
                 "- Được đánh dấu duyệt tự động theo ủy quyền; không phải kết quả nghe.", "",
                 "## Training", "",
                 f"- Dừng vì: {tr.get('stop_reason')} ở optimizer step {tr.get('step')} (kế hoạch: {st.get('run', {}).get('max_steps')} step, "
                 f"{st.get('run', {}).get('steps_per_epoch')} step mỗi epoch).",
                 f"- Thời gian train cộng dồn: {tr.get('train_seconds')} giây. VRAM đỉnh: {tr.get('vram_peak_mib')} MiB.", "",
                 "## Checkpoint được chọn", "",
                 f"- Step {sel.get('step')}, tiêu chí: {sel.get('basis')}; validation loss {sel.get('validation_loss')}.",
                 f"- Các ứng viên (step: validation loss): {sel.get('candidates')}",
                 "- Validation loss không phải bằng chứng nhạc hay hơn. Các checkpoint mốc khác còn trong thư mục run; "
                 "xuất cái khác bằng `export_model.py export`.", "",
                 "## Chưa làm", "",
                 "- Chưa render và chưa ai nghe model này. Chưa nạp thử vào engine inference.",
                 "- Gói mang nhãn chưa đánh giá (EVALUATION.md)."]
        out = self.dir / "REPORT.md"
        out.write_text("\n".join(lines) + "\n", encoding="utf-8")
        return out


def _env() -> dict[str, str]:
    import os
    return {**os.environ, "PYTHONUNBUFFERED": "1",
            "PYTHONPATH": os.pathsep.join([str(WORKSPACE / "src"), os.environ.get("PYTHONPATH", "")]).rstrip(os.pathsep)}
