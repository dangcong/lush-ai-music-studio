"""Command line for every script in `scripts/`. Each script is a thin entry into one sub-parser here.

Commands that use the GPU (`train preprocess`, `train start`, `train resume`) refuse to run without
`--confirm-gpu`, so nothing touches the GPU by accident.
"""
from __future__ import annotations

import argparse
import importlib
import json
import os
import shutil
import subprocess
import sys
import time
from datetime import datetime, timedelta
from pathlib import Path
from typing import Any, Optional

from . import captions, checkpoint, dataset, doctor, evaluate, export, model, run as runs, session
from .config import WORKSPACE, Project, load_project
from .util import LushTrainingError, read_json

DEFAULT_BACKEND = "lush_training.acestep_backend:AceStepDecoderBackend"


def out(obj: Any, as_json: bool = False) -> None:
    if as_json or not isinstance(obj, str):
        print(json.dumps(obj, indent=2, ensure_ascii=False, default=str))
    else:
        print(obj)


# ---------- doctor / model ----------

def cmd_doctor(a: argparse.Namespace) -> int:
    project, error = None, None
    try:
        project = load_project(a.project)
    except LushTrainingError as e:
        error = str(e)
    report = doctor.run_doctor(project, error)
    out(report if a.json else doctor.format_report(report))
    return 1 if report["status"] == doctor.FAIL else 0


def cmd_inspect(a: argparse.Namespace) -> int:
    project = load_project(a.project)
    path = Path(a.checkpoint)
    model_dir = path if path.is_dir() else project.checkpoints_dir / a.checkpoint
    report = model.inspect_checkpoint(model_dir, with_hash=a.hash)
    if a.method and report["compatibility"]["compatible"]:
        report["plan"] = model.trainable_plan(report, a.method, a.pattern or None, model.tensor_counts(model_dir))
    if a.json:
        out(report, True)
        return 0
    c = report["compatibility"]
    print(f"Checkpoint: {report['name']} ({report['weights_bytes'] / 2**30:.2f} GiB, {report['total_tensors']} tensor, "
          f"{report['total_parameters']:,} tham số)")
    if report["weights_sha256"]:
        print(f"SHA-256: {report['weights_sha256']}")
    for name, comp in report["components"].items():
        print(f"  {name:<24} {comp['parameters']:>16,} tham số  {comp['bytes'] / 2**30:6.2f} GiB  {','.join(comp['dtypes'])}")
    print(f"Backend: {c['backend'] or 'KHÔNG TƯƠNG THÍCH'}; method hỗ trợ: {c['methods']}")
    for r in c["reasons"]:
        print(f"  [FAIL] {r}")
    for w in c["warnings"]:
        print(f"  [WARN] {w}")
    print(c["note"])
    if "plan" in report:
        p = report["plan"]
        print(f"Method `{p['method']}`: {p['description']}")
        print(f"  Train: {p['trainable_parameters']:,} / {p['total_parameters']:,} tham số trong file model "
              f"({p['trainable_fraction_of_model_file']:.2%}). Đóng băng: {', '.join(p['frozen_components'])}")
    return 0 if c["compatible"] else 1


# ---------- dataset ----------

def cmd_dataset(a: argparse.Namespace) -> int:
    project = load_project(a.project)
    cfg = dataset.load_dataset_config(a.config)
    if a.action == "scan":
        project.ensure_layout()
        scanned = dataset.scan(cfg, project)
        print(dataset.scan_summary(scanned, dataset.load_reviews(project, cfg["name"]), cfg["vocal_flag_threshold"]))
        print(f"Chi tiết: {dataset.work_dir(project, cfg['name']) / 'scan.json'}")
    elif a.action == "review":
        scanned = read_json(dataset.work_dir(project, cfg["name"]) / "scan.json")
        matches = [t for t in scanned["tracks"] if t["rel"] == a.file or t["sha256"].startswith(a.file)]
        if len(matches) != 1:
            raise LushTrainingError(f"`{a.file}` khớp {len(matches)} track; dùng đường dẫn tương đối như trong báo cáo scan")
        dataset.record_review(project, cfg["name"], matches[0]["sha256"], a.decision, a.by, a.note or "")
        print(f"Đã ghi: {matches[0]['rel']} -> {a.decision} (người nghe: {a.by})")
    else:
        manifest = dataset.build(cfg, project, a.version)
        print(f"Đã khóa dataset {cfg['name']}/{a.version}:")
        for split, t in manifest["totals"].items():
            print(f"  {split:<10} {t['tracks']} track, {t['groups']} nhóm, {t['segments']} segment, {t['hours']} giờ")
        vs = manifest["vocal_screening"]
        print(f"  Vocal: phương pháp `{vs['method']}`, {vs['tracks_not_screened']} track chưa sàng lọc, "
              f"{vs['tracks_confirmed_by_listening']} track người nghe xác nhận.")
    return 0


def _vdir(project: Project, a: argparse.Namespace) -> Path:
    vdir = dataset.version_dir(project, a.dataset, a.version)
    dataset.load_manifest(vdir)
    return vdir


def cmd_analyze(a: argparse.Namespace) -> int:
    project = load_project(a.project)
    vdir = _vdir(project, a)
    manifest, _ = dataset.load_manifest(vdir)
    done = 0
    missing: set[str] = set()
    for seg in manifest["segments"]:
        result = captions.analyze_segment(vdir, seg)
        missing.update(result["not_available"])
        done += 1
    print(f"Đã phân tích {done} segment (kết quả cache theo SHA-256 của segment) tại {vdir / 'analysis'}.")
    for m in sorted(missing):
        print(f"  Không có: {m}")
    return 0


def cmd_captions(a: argparse.Namespace) -> int:
    project = load_project(a.project)
    vdir = _vdir(project, a)
    if a.action == "init":
        print(captions.init_work(vdir, analyze=not a.no_analyze))
    elif a.action == "validate":
        report = captions.validate_work(vdir)
        if a.json:
            out(report, True)
        else:
            counts = {k: len(v) for k, v in report["by_status"].items()}
            print(f"Cần caption: {report['segments_needing_caption']} segment. Trạng thái: {counts}. "
                  f"Thiếu: {len(report['missing'])}. Lỗi: {len(report['errors'])}. Cảnh báo: {len(report['warnings'])}.")
            for sid, errs in list(report["errors"].items())[:40]:
                print(f"  [LỖI] {sid}: {'; '.join(errs)}")
            if report["by_status"]["needs_listening"]:
                print("  Cần bạn nghe lại: " + ", ".join(report["by_status"]["needs_listening"][:40]))
            print("Sẵn sàng khóa." if report["ready_to_lock"] else "Chưa khóa được.")
        return 0 if not report["errors"] else 1
    elif a.action == "set":
        meta = json.loads(a.metadata) if a.metadata else None
        cap = captions.set_fields(vdir, a.segment, by=a.by, human=a.human, metadata=meta, caption_text=a.caption,
                                  status=a.status, note=a.note or "")
        print(f"{a.segment}: status {cap['status']}")
    else:
        info = captions.lock(vdir, a.caption_version)
        print(f"Đã khóa caption version `{a.caption_version}`: {info['segments']}, sha256 {info['captions_sha256'][:16]}…")
    return 0


def cmd_studio_export(a: argparse.Namespace) -> int:
    from .studio_export import export_studio
    project = load_project(a.project)
    vdir = _vdir(project, a)
    result = export_studio(vdir, a.caption_version, a.studio_root, a.key)
    print(f"Đã xuất `{result['key']}`: {result['train_samples']} mẫu train tại {result['dir']}.")
    print(f"  Validation giữ lại để đánh giá: {len(result['excluded_validation'])} segment.")
    if result["split_tracks"]:
        print(f"  {len(result['split_tracks'])} bài dài hơn một mẫu (đã tách theo segment.seconds): "
              + ", ".join(result["split_tracks"]))
    print("  Bước tiếp: restart lush-worker để node báo dataset mới (cần người dùng đồng ý).")
    return 0


# ---------- training ----------

def preflight(project: Project, run_dir: Path) -> list[dict[str, Any]]:
    """Checks before a session. Reads only; uses no GPU memory."""
    info = runs.load_run(run_dir)
    cfg = info["config"]
    checks: list[dict[str, Any]] = []

    def add(name: str, ok: bool, detail: str) -> None:
        checks.append({"name": name, "ok": ok, "detail": detail})
    try:
        vdir = dataset.version_dir(project, cfg["dataset"]["name"], cfg["dataset"]["version"])
        _, digest = dataset.load_manifest(vdir)
        lock = captions.load_lock(vdir, cfg["dataset"]["caption_version"])
        same = digest == info["dataset"]["manifest_sha256"] and lock["captions_sha256"] == info["dataset"]["captions_sha256"]
        add("dataset", same, "khớp run.json" if same else "dataset hoặc caption đã khác run.json")
    except LushTrainingError as e:
        add("dataset", False, str(e))
    ok = runs.parent_sha_matches(project, info)
    add("parent_checkpoint", ok, "SHA-256 khớp run.json" if ok else "thiếu hoặc khác SHA-256 trong run.json")
    for split in ("train", "validation"):
        need = info["dataset"]["segments"][split]
        if split == "validation" and cfg["validation"]["every_steps"] <= 0:
            continue
        tdir = runs.tensor_dir(project, cfg, split)
        have = len(list(tdir.glob("*.pt"))) if tdir.is_dir() else 0
        add(f"tensors_{split}", have == need, f"{have}/{need} tensor tại {tdir}" + ("" if have == need else " (chạy `train.py preprocess`)"))
    from .acestep_probe import probe_repo
    if project.acestep_repo is None or project.acestep_python is None:
        add("acestep", False, "thiếu acestep.repo_dir hoặc acestep.python trong project.yaml")
    else:
        probe = probe_repo(project.acestep_repo, project.acestep_commit)
        add("acestep", probe["ok"] and probe["commit"] == info["code"]["acestep_commit"],
            "; ".join(probe["problems"]) or f"commit {str(probe['commit'])[:12]}")
        if project.acestep_python.is_file():
            needed = "import yaml, numpy, soundfile, torch, safetensors, transformers"
            if cfg["optimizer"]["type"] == "adamw8bit":
                needed += ", bitsandbytes"
            proc = subprocess.run([str(project.acestep_python), "-c", needed], capture_output=True, text=True, timeout=300)
            add("trainer_python", proc.returncode == 0, "đủ gói" if proc.returncode == 0 else proc.stderr.strip().splitlines()[-1])
        else:
            add("trainer_python", False, f"không có {project.acestep_python}")
    free_gb = shutil.disk_usage(project.data_root).free / 2**30
    need_gb = cfg["preflight"]["min_free_disk_gb"]
    add("disk", free_gb >= need_gb, f"trống {free_gb:.1f} GiB, cấu hình yêu cầu {need_gb} GiB")
    gpus = doctor.query_gpus()
    if not gpus:
        add("gpu", False, "nvidia-smi không báo GPU nào")
    else:
        g = gpus[0]
        free = (g["memory_total_mib"] or 0) - (g["memory_used_mib"] or 0)
        need = cfg["preflight"]["min_free_vram_mib"]
        add("gpu", need is None or free >= need,
            f"{g['name']}: trống {free} MiB / {g['memory_total_mib']} MiB" + (f", cấu hình yêu cầu {need} MiB" if need else
                                                                             " (chưa đặt ngưỡng; việc vừa VRAM hay không phải đo)"))
    alive = runs.trainer_pid(run_dir)
    add("not_running", alive is None, "không có trainer nào đang chạy run này" if alive is None else f"trainer pid {alive} đang chạy")
    latest, rejected = checkpoint.latest_valid(run_dir)
    has_any = bool(checkpoint.list_steps(run_dir))
    add("resume_checkpoint", latest is not None or not has_any,
        f"resume từ {latest.name}" if latest else ("run mới, bắt đầu từ step 0" if not has_any else
                                                   f"không còn checkpoint hợp lệ: {rejected}"))
    return checks


def parse_stop_at(value: Optional[str]) -> Optional[float]:
    if not value:
        return None
    hour, minute = (int(x) for x in value.split(":"))
    now = datetime.now()
    target = now.replace(hour=hour, minute=minute, second=0, microsecond=0)
    if target <= now:
        target += timedelta(days=1)
    return target.timestamp()


def load_backend(spec: str, project: Project, run_dir: Path) -> Any:
    module, _, attr = spec.partition(":")
    return getattr(importlib.import_module(module), attr)(project, run_dir)


def cmd_train(a: argparse.Namespace) -> int:
    project = load_project(a.project)
    if a.action == "create":
        project.ensure_layout()
        run_dir = runs.create_run(project, runs.load_training_config(a.config))
        info = runs.load_run(run_dir)
        t = info["training"]
        print(f"Đã tạo run {info['run_id']} tại {run_dir}")
        print(f"  Method: {t['method']} — {t['description']}")
        print(f"  Train {t['trainable_parameters']:,} / {t['total_parameters']:,} tham số; đóng băng: {', '.join(t['frozen_components'])}")
        print("  Chưa chạy gì trên GPU. Tiếp theo: `train.py preflight`, rồi `train.py preprocess` và `train.py start` khi được phê duyệt.")
        return 0
    run_dir = runs.find_run(project, a.run)
    info = runs.load_run(run_dir)
    if a.action == "preflight":
        checks = preflight(project, run_dir)
        for c in checks:
            print(f"[{'OK' if c['ok'] else 'FAIL'}] {c['name']}: {c['detail']}")
        return 0 if all(c["ok"] for c in checks) else 1
    if a.action == "stop":
        if runs.trainer_pid(run_dir) is None:
            print("Không có trainer nào đang chạy run này.")
            return 0
        runs.request_stop(run_dir)
        print("Đã yêu cầu dừng an toàn: trainer kết thúc optimizer step đang chạy, ghi checkpoint rồi thoát. "
              "Xem `status.py`. Lệnh này không kill tiến trình.")
        return 0
    if a.action in ("preprocess", "start", "resume") and not a.confirm_gpu:
        raise LushTrainingError("Lệnh này dùng GPU. Chỉ chạy khi người dùng đã phê duyệt, và thêm --confirm-gpu.")
    if a.action == "preprocess":
        from .acestep_backend import preprocess_command, write_engine_json
        cfg = info["config"]
        vdir = dataset.version_dir(project, cfg["dataset"]["name"], cfg["dataset"]["version"])
        for split in ("train", "validation"):
            need = info["dataset"]["segments"][split]
            if need == 0:
                continue
            tdir = runs.tensor_dir(project, cfg, split)
            tdir.mkdir(parents=True, exist_ok=True)
            engine_json = write_engine_json(vdir, cfg["dataset"]["caption_version"], split, tdir)
            started = time.time()
            with open(tdir / "preprocess.log", "ab") as log:
                code = subprocess.run(preprocess_command(project, cfg, split, engine_json, tdir), cwd=str(project.data_root),
                                      stdout=log, stderr=subprocess.STDOUT).returncode
            have = len(list(tdir.glob("*.pt")))
            runs.event(run_dir, "preprocess", split=split, exit_code=code, tensors=have, expected=need,
                       seconds=round(time.time() - started, 1))
            print(f"{split}: {have}/{need} tensor, exit {code}, {time.time() - started:.0f} giây. Log: {tdir / 'preprocess.log'}")
            if code != 0 or have != need:
                return 1
        return 0
    if a.action in ("start", "resume"):
        has_ckpt = bool(checkpoint.list_steps(run_dir))
        if a.action == "start" and has_ckpt:
            raise LushTrainingError("run này đã có checkpoint; dùng `resume` để tiếp tục (không bắt đầu lại từ step 0)")
        if a.action == "resume" and not has_ckpt:
            raise LushTrainingError("run này chưa có checkpoint nào để resume; dùng `start`")
        if a.backend == DEFAULT_BACKEND:
            failed = [c for c in preflight(project, run_dir) if not c["ok"]]
            if failed:
                raise LushTrainingError("preflight không đạt: " + "; ".join(f"{c['name']}: {c['detail']}" for c in failed))
        worker = ["-m", "lush_training.cli", "train", "_worker", "--run", str(run_dir), "--backend", a.backend]
        if a.project:
            worker += ["--project", str(a.project)]
        if a.session_hours is not None:
            worker += ["--session-hours", str(a.session_hours)]
        if a.stop_at:
            worker += ["--stop-at", a.stop_at]
        if a.foreground:
            return _worker(project, run_dir, a)
        python = str(project.acestep_python) if a.backend == DEFAULT_BACKEND and project.acestep_python else sys.executable
        env = {**os.environ, "PYTHONPATH": os.pathsep.join([str(WORKSPACE / "src"), os.environ.get("PYTHONPATH", "")]).rstrip(os.pathsep),
               "PYTHONUNBUFFERED": "1"}
        kwargs: dict[str, Any] = {}
        if os.name == "nt":
            kwargs["creationflags"] = subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP
        else:
            kwargs["start_new_session"] = True
        with open(run_dir / "trainer.log", "ab") as log:
            proc = subprocess.Popen([python, *worker], cwd=str(project.data_root), env=env, stdin=subprocess.DEVNULL,
                                    stdout=log, stderr=subprocess.STDOUT, **kwargs)
        print(f"Trainer chạy nền, pid {proc.pid}. Log: {run_dir / 'trainer.log'}. Theo dõi: scripts/status.py --run {info['run_id']}")
        print("Tiến trình tách khỏi terminal này và khỏi Claude Code. Nó KHÔNG tự chạy lại sau reboot, và có sống qua "
              "logout hay không thì chưa kiểm chứng trên máy training.")
        return 0
    return _worker(project, run_dir, a)


def _worker(project: Project, run_dir: Path, a: argparse.Namespace) -> int:
    backend = load_backend(a.backend, project, run_dir)
    code = session.run_session(run_dir, backend, session_hours=a.session_hours, stop_at=parse_stop_at(a.stop_at))
    print(f"exit {code}: {runs.read_status(run_dir).get('stop_reason')}")
    return code


def cmd_status(a: argparse.Namespace) -> int:
    project = load_project(a.project)
    if not a.run:
        root = project.dir("runs")
        rows = [(p.name, runs.describe(p)) for p in sorted(root.iterdir()) if (p / "run.json").is_file()] if root.is_dir() else []
        if not rows:
            print(f"Chưa có run nào trong {root}.")
        for name, d in rows:
            print(f"{name}: {d['status'].get('state')} step {d['status'].get('step')}/{d['max_steps']} "
                  f"({d['latest_valid_checkpoint'] or 'chưa có checkpoint'})")
        return 0
    run_dir = runs.find_run(project, a.run)
    d = runs.describe(run_dir)
    if a.log:
        log = run_dir / "trainer.log"
        print("\n".join(log.read_text(encoding="utf-8", errors="replace").splitlines()[-a.log:]) if log.is_file() else "(chưa có trainer.log)")
        return 0
    if a.json:
        out(d, True)
        return 0
    s = d["status"]
    print(f"Run {d['run_id']} — {s.get('state')}" + (f" ({s.get('stop_reason')})" if s.get("stop_reason") else ""))
    if s.get("note"):
        print(f"  {s['note']}")
    if s.get("error"):
        print(f"  Lỗi: {s['error']}")
    print(f"  Method {d['method']}: {d['trainable_parameters']:,} / {d['total_parameters']:,} tham số được train")
    print(f"  Optimizer step {s.get('step')}/{d['max_steps']}; epoch (lượt qua tập train) {s.get('epoch', 'chưa có dữ liệu')}")
    print(f"  Train loss gần nhất: {s.get('last_loss', 'chưa có dữ liệu')}; validation loss: {s.get('validation_loss', 'chưa có dữ liệu')}; "
          f"learning rate: {s.get('learning_rate', 'chưa có dữ liệu')}")
    print(f"  Thời gian train cộng dồn: {s.get('train_seconds_total', 0)} giây; {s.get('seconds_per_step', 'chưa có dữ liệu')} giây/step; "
          f"VRAM đỉnh: {s.get('vram_peak_mib', 'chưa có dữ liệu')} MiB")
    eta = s.get("eta_seconds_to_max_steps")
    print(f"  ETA tới max_steps: {str(timedelta(seconds=eta)) if eta is not None else 'chưa đủ số đo'}")
    print(f"  Trainer pid: {d['trainer_pid'] or 'không chạy'}; yêu cầu dừng: {'có' if d['stop_requested'] else 'không'}")
    print(f"  Checkpoint hợp lệ mới nhất: {d['latest_valid_checkpoint'] or 'chưa có'}; tất cả: {d['checkpoints']}")
    for r in d["rejected_checkpoints"]:
        print(f"  [HỎNG] {r['checkpoint']}: {r['problems']}")
    return 0


# ---------- evaluate / export ----------

def cmd_evaluate(a: argparse.Namespace) -> int:
    project = load_project(a.project)
    run_dir = runs.find_run(project, a.run)
    info = runs.load_run(run_dir)
    eval_dir = project.dir("eval") / info["run_id"] / a.checkpoint
    cfg = info["config"]["dataset"]
    vdir = dataset.version_dir(project, cfg["name"], cfg["version"])
    if a.action == "plan":
        plan = evaluate.make_plan(a.prompts, eval_dir, run_id=info["run_id"], checkpoint_name=a.checkpoint)
        print(f"Kế hoạch render: {len(plan['items'])} mục mỗi phía tại {eval_dir}. {plan['render']}")
        print("Render cần engine inference trên GPU: workspace này không tự render.")
    elif a.action == "check":
        plan = read_json(eval_dir / "plan.json")
        for side in evaluate.SIDES:
            rows = evaluate.check_side(eval_dir, side, plan)
            bad = [r for r in rows if r["problems"]]
            print(f"{side}: {len(rows) - len(bad)}/{len(rows)} đạt kiểm tra kỹ thuật")
            for r in bad:
                print(f"  {r['file']}: {'; '.join(r['problems'])}")
    elif a.action == "ab":
        print(evaluate.make_ab(eval_dir, seed=a.seed))
        print(f"Phiếu nghe: {eval_dir / 'ab' / 'sheet.csv'}. Không mở ab_key.json trước khi nghe xong.")
    else:
        print(f"Báo cáo: {evaluate.write_report(eval_dir, vdir)}")
    return 0


def cmd_export(a: argparse.Namespace) -> int:
    project = load_project(a.project)
    if a.action == "verify":
        problems = export.verify_package(Path(a.package))
        print("\n".join(problems) if problems else "OK: gói nguyên vẹn và đủ thành phần bắt buộc.")
        return 1 if problems else 0
    run_dir = runs.find_run(project, a.run)
    info = runs.load_run(run_dir)
    report = Path(a.evaluation) if a.evaluation else project.dir("eval") / info["run_id"] / a.checkpoint / "EVALUATION.md"
    target = export.export_package(project, run_dir, a.checkpoint, a.name, report)
    manifest = read_json(target / "MANIFEST.json")
    print(f"Đã xuất gói inference: {target} ({len(manifest['files'])} file).")
    print("Có báo cáo đánh giá." if manifest["evaluated"] else "CẢNH BÁO: gói chưa có báo cáo đánh giá.")
    print(f"Hướng dẫn import và rollback: {target / 'IMPORT.md'}. Workspace không tự chép gói đi đâu.")
    return 0


def cmd_auto(a: argparse.Namespace) -> int:
    from .autopilot import Autopilot, load_job
    project = load_project(a.project)
    job = load_job(a.job)
    work = project.data_root / "autopilot" / job["name"]
    if a.action == "status":
        state = work / "state.json"
        report = work / "REPORT.md"
        if not state.is_file():
            print("Job này chưa chạy lần nào.")
            return 0
        stages = read_json(state)["stages"]
        print("Các bước đã xong: " + (", ".join(stages) or "chưa có"))
        run_dir = stages.get("run", {}).get("run_dir")
        if run_dir and (Path(run_dir) / "run.json").is_file():
            print(runs.write_progress_report(Path(run_dir)).read_text(encoding="utf-8"))
        if report.is_file():
            print(report.read_text(encoding="utf-8"))
        return 0
    if a.detach:
        work.mkdir(parents=True, exist_ok=True)
        cmd = [sys.executable, "-m", "lush_training.cli", "auto", "run", "--job", str(Path(a.job).resolve()), "--backend", a.backend]
        if a.project:
            cmd += ["--project", str(Path(a.project).resolve())]
        env = {**os.environ, "PYTHONUNBUFFERED": "1",
               "PYTHONPATH": os.pathsep.join([str(WORKSPACE / "src"), os.environ.get("PYTHONPATH", "")]).rstrip(os.pathsep)}
        kwargs: dict[str, Any] = ({"creationflags": subprocess.DETACHED_PROCESS | subprocess.CREATE_NEW_PROCESS_GROUP}
                                  if os.name == "nt" else {"start_new_session": True})
        with open(work / "autopilot.log", "ab") as log:
            proc = subprocess.Popen(cmd, cwd=str(project.data_root), env=env, stdin=subprocess.DEVNULL, stdout=log,
                                    stderr=subprocess.STDOUT, **kwargs)
        print(f"Autopilot chạy nền, pid {proc.pid}. Log: {work / 'autopilot.log'}. Xem tiến độ: scripts/autopilot.py status --job {a.job}")
        return 0
    result = Autopilot(project, a.job, project_path=Path(a.project).resolve() if a.project else None, backend=a.backend).run()
    print(f"Báo cáo: {work / 'REPORT.md'}")
    print(f"Gói để test: {result['package']}" + ("" if result["finished"] else " (dừng theo giờ; chạy lại cùng job để train tiếp)"))
    return 0


# ---------- parser ----------

def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(prog="lush-training")
    sub = p.add_subparsers(dest="command", required=True)

    def add(name: str, func: Any) -> argparse.ArgumentParser:
        sp = sub.add_parser(name)
        sp.add_argument("--project", type=Path, default=None, help="configs/project.yaml (mặc định)")
        sp.set_defaults(func=func)
        return sp

    d = add("doctor", cmd_doctor)
    d.add_argument("--json", action="store_true")

    i = add("inspect", cmd_inspect)
    i.add_argument("checkpoint", help="tên thư mục trong vùng pretrained, hoặc đường dẫn")
    i.add_argument("--hash", action="store_true", help="tính SHA-256 của weights (vài phút với file nhiều GB)")
    i.add_argument("--method", choices=sorted(model.TRAINING_METHODS))
    i.add_argument("--pattern", action="append", help="regex cho partial_decoder")
    i.add_argument("--json", action="store_true")

    ds = add("dataset", cmd_dataset)
    ds.add_argument("action", choices=["scan", "review", "build"])
    ds.add_argument("--config", type=Path, required=True)
    ds.add_argument("--version")
    ds.add_argument("--file")
    ds.add_argument("--decision", choices=dataset.VOCAL_DECISIONS)
    ds.add_argument("--by", default="")
    ds.add_argument("--note")

    an = add("analyze", cmd_analyze)
    cp = add("captions", cmd_captions)
    cp.add_argument("action", choices=["init", "validate", "set", "lock"])
    for sp in (an, cp):
        sp.add_argument("--dataset", required=True)
        sp.add_argument("--version", required=True)
    cp.add_argument("--no-analyze", action="store_true")
    cp.add_argument("--json", action="store_true")
    cp.add_argument("--segment")
    cp.add_argument("--metadata", help='JSON, ví dụ {"genre": "lo-fi", "tempo_bpm": null}')
    cp.add_argument("--caption")
    cp.add_argument("--status", choices=captions.STATUSES)
    cp.add_argument("--by", default="")
    cp.add_argument("--human", action="store_true", help="thay đổi do chính người dùng quyết định")
    cp.add_argument("--note")
    cp.add_argument("--caption-version")

    se = add("studio-export", cmd_studio_export)
    se.add_argument("--dataset", required=True)
    se.add_argument("--version", required=True)
    se.add_argument("--caption-version", required=True)
    se.add_argument("--studio-root", type=Path, required=True, help="thư mục datasets/ mà worker Studio đọc")
    se.add_argument("--key", required=True, help="<tên>/<version>, ví dụ lofi-pilot/v1")

    t = add("train", cmd_train)
    t.add_argument("action", choices=["create", "preflight", "preprocess", "start", "resume", "stop", "_worker"])
    t.add_argument("--config", type=Path)
    t.add_argument("--run")
    t.add_argument("--confirm-gpu", action="store_true")
    t.add_argument("--session-hours", type=float)
    t.add_argument("--stop-at", help="HH:MM giờ máy; dừng an toàn ở mốc này")
    t.add_argument("--foreground", action="store_true")
    t.add_argument("--backend", default=DEFAULT_BACKEND, help=argparse.SUPPRESS)

    s = add("status", cmd_status)
    s.add_argument("--run")
    s.add_argument("--log", type=int, help="in N dòng cuối của trainer.log")
    s.add_argument("--json", action="store_true")

    e = add("evaluate", cmd_evaluate)
    e.add_argument("action", choices=["plan", "check", "ab", "report"])
    e.add_argument("--run", required=True)
    e.add_argument("--checkpoint", required=True)
    e.add_argument("--prompts", type=Path)
    e.add_argument("--seed", type=int, default=0)

    au = add("auto", cmd_auto)
    au.add_argument("action", choices=["run", "status"])
    au.add_argument("--job", type=Path, required=True)
    au.add_argument("--detach", action="store_true", help="chạy nền, tách khỏi terminal")
    au.add_argument("--backend", default=DEFAULT_BACKEND, help=argparse.SUPPRESS)

    x = add("export", cmd_export)
    x.add_argument("action", choices=["export", "verify"])
    x.add_argument("--run")
    x.add_argument("--checkpoint")
    x.add_argument("--name")
    x.add_argument("--evaluation")
    x.add_argument("--package")
    return p


def main(argv: Optional[list[str]] = None) -> int:
    for stream in (sys.stdout, sys.stderr):
        if hasattr(stream, "reconfigure"):
            stream.reconfigure(encoding="utf-8", errors="replace")
    args = build_parser().parse_args(argv)
    try:
        return int(args.func(args) or 0)
    except LushTrainingError as e:
        print(f"LỖI: {e}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
