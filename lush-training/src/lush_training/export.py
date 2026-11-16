"""Build an inference package from a checkpoint: a model directory the engine loads like any other
variant, plus manifest, provenance, evaluation report and import instructions.

The merged `model.safetensors` is the parent file with the trained decoder tensors swapped in (cast
to the parent's dtype). Every other tensor is copied byte for byte and then verified. Needs no
torch and no GPU. Nothing is uploaded or installed anywhere: copying the package is the user's step.
"""
from __future__ import annotations

import hashlib
import os
import shutil
from pathlib import Path
from typing import Any, Iterator, Optional

from . import checkpoint, model, safetensors_io
from .config import Project
from .run import load_run, parent_sha_matches
from .util import (LushTrainingError, atomic_write_json, file_manifest, now_iso, read_json, remove_tree, verify_manifest)

DECODER_FILE = "decoder.safetensors"
SHARED_DEPENDENCIES = ("vae", "Qwen3-Embedding-0.6B")

VERIFY_SCRIPT = '''"""Verify this package against MANIFEST.json. Standard library only: python verify_package.py"""
import hashlib, json, sys
from pathlib import Path

root = Path(__file__).resolve().parent
manifest = json.loads((root / "MANIFEST.json").read_text(encoding="utf-8"))
bad = []
for rel, meta in manifest["files"].items():
    p = root / rel
    if not p.is_file():
        bad.append(f"missing: {rel}")
        continue
    h = hashlib.sha256()
    with open(p, "rb") as f:
        while block := f.read(1 << 20):
            h.update(block)
    if p.stat().st_size != meta["bytes"] or h.hexdigest() != meta["sha256"]:
        bad.append(f"differs: {rel}")
print("\\n".join(bad) if bad else f"OK: {len(manifest['files'])} files match MANIFEST.json")
sys.exit(1 if bad else 0)
'''


def merge_weights(parent_file: Path, trained_file: Path, out_file: Path) -> dict[str, Any]:
    """Write `out_file` = parent with trained `decoder.*` tensors. Returns what changed."""
    parent = safetensors_io.read_header(parent_file)
    trained = safetensors_io.read_header(trained_file)
    parent_decoder = {n for n, t in parent.tensors.items() if model.component_of(n) == "decoder"
                      and t.dtype in ("F32", "BF16", "F16")}
    extra = sorted(set(trained.tensors) - parent_decoder)
    missing = sorted(parent_decoder - set(trained.tensors))
    if extra or missing:
        raise LushTrainingError(f"checkpoint không khớp decoder của checkpoint gốc: thừa {extra[:5]}, thiếu {missing[:5]}")
    for name in trained.tensors:
        if trained.tensors[name].shape != parent.tensors[name].shape:
            raise LushTrainingError(f"tensor `{name}` khác shape với checkpoint gốc")
        if trained.tensors[name].dtype != "F32":
            raise LushTrainingError(f"tensor `{name}` trong checkpoint không phải F32")
    ordered = sorted(parent.tensors.values(), key=lambda t: t.begin)
    stats = {"decoder_tensors": len(parent_decoder), "decoder_tensors_changed": 0, "frozen_tensors": 0}

    def chunks() -> Iterator[bytes]:
        with open(parent_file, "rb") as pf, open(trained_file, "rb") as tf:
            for info in ordered:
                original = safetensors_io.read_tensor_bytes(pf, parent, info)
                if info.name in trained.tensors:
                    data = safetensors_io.cast_f32_bytes(
                        safetensors_io.read_tensor_bytes(tf, trained, trained.tensors[info.name]), info.dtype, info.name)
                    stats["decoder_tensors_changed"] += data != original
                    yield data
                else:
                    stats["frozen_tensors"] += 1
                    yield original

    safetensors_io.write_safetensors(out_file, [(t.name, t.dtype, t.shape, t.nbytes) for t in ordered], chunks(),
                                     {**parent.metadata, "lush_parent_weights": parent_file.parent.name})
    # Independent check of the written file: frozen tensors must be byte-identical to the parent.
    written = safetensors_io.read_header(out_file)
    if set(written.tensors) != set(parent.tensors):
        raise LushTrainingError("file xuất ra không có cùng tập tensor với checkpoint gốc")
    with open(parent_file, "rb") as pf, open(out_file, "rb") as of:
        for info in ordered:
            if info.name in trained.tensors:
                continue
            a = hashlib.sha256(safetensors_io.read_tensor_bytes(pf, parent, info)).digest()
            b = hashlib.sha256(safetensors_io.read_tensor_bytes(of, written, written.tensors[info.name])).digest()
            if a != b:
                raise LushTrainingError(f"tensor đóng băng `{info.name}` bị thay đổi trong file xuất ra")
    stats["frozen_tensors_verified_identical"] = stats["frozen_tensors"]
    return stats


def export_package(project: Project, run_dir: Path, checkpoint_name: str, export_name: str,
                   evaluation_report: Optional[Path] = None) -> Path:
    info = load_run(run_dir)
    ckpt = checkpoint.checkpoints_dir(run_dir) / checkpoint_name
    if not (ckpt / checkpoint.MANIFEST).is_file():
        raise LushTrainingError(f"không có checkpoint hợp lệ: {ckpt}")
    cmanifest = read_json(ckpt / checkpoint.MANIFEST)
    problems = verify_manifest(ckpt, cmanifest["files"])
    if problems or DECODER_FILE not in cmanifest["files"]:
        raise LushTrainingError(f"checkpoint {checkpoint_name} không qua kiểm tra: {problems or 'thiếu ' + DECODER_FILE}")
    if not parent_sha_matches(project, info):
        raise LushTrainingError("checkpoint gốc trên đĩa không còn khớp SHA-256 trong run.json")
    if not model.NAME_OK.match(export_name):
        raise LushTrainingError(f"tên gói không hợp lệ: {export_name!r}")
    parent_dir = project.checkpoints_dir / info["parent"]["name"]
    target = project.dir("exports") / export_name
    if target.exists():
        raise LushTrainingError(f"gói đã tồn tại, không ghi đè: {target}")
    partial = target.with_name(target.name + ".partial")
    remove_tree(partial)
    model_out = partial / "model" / export_name
    model_out.mkdir(parents=True)
    try:
        for p in sorted(parent_dir.iterdir()):
            if p.is_file() and p.name != model.WEIGHTS_FILE:
                shutil.copy2(p, model_out / p.name)
        stats = merge_weights(parent_dir / model.WEIGHTS_FILE, ckpt / DECODER_FILE, model_out / model.WEIGHTS_FILE)
        exported = model.inspect_checkpoint(model_out)
        if exported["total_parameters"] != info["parent"]["total_parameters"]:
            raise LushTrainingError("model xuất ra có số tham số khác checkpoint gốc")
        progress = read_json(ckpt / checkpoint.PROGRESS)
        missing_deps = [d for d in SHARED_DEPENDENCIES if not (project.checkpoints_dir / d).is_dir()]
        atomic_write_json(partial / "PROVENANCE.json", {
            "export_name": export_name, "exported_at": now_iso(), "instrumental_only": True,
            "run_id": info["run_id"], "checkpoint": checkpoint_name, "optimizer_step": progress.get("step"),
            "train_seconds": progress.get("train_seconds"),
            "parent": {k: info["parent"][k] for k in ("name", "weights_sha256", "total_parameters", "license", "source")},
            "training": info["training"], "weight_merge": stats,
            "dataset": info["dataset"], "config": info["config"], "code": info["code"],
            "engine": {"name": "ACE-Step 1.5", "commit": info["code"]["acestep_commit"],
                       "model_directory": f"model/{export_name}", "weights_dtype": exported["components"]["decoder"]["dtypes"]},
            "requires_on_inference_machine": [
                {"name": d, "where": f"<ACE-Step>/checkpoints/{d}", "included": False,
                 "present_on_training_machine": d not in missing_deps} for d in SHARED_DEPENDENCIES],
            "license_note": "Giấy phép của model xuất ra kế thừa từ checkpoint gốc và quyền sử dụng dataset; "
                            "cả hai do người dùng khai báo, workspace không tự xác minh.",
        })
        if evaluation_report and Path(evaluation_report).is_file():
            shutil.copy2(evaluation_report, partial / "EVALUATION.md")
            evaluated = True
        else:
            (partial / "EVALUATION.md").write_text(
                "# Evaluation\n\nCHƯA CÓ báo cáo đánh giá cho checkpoint này. Gói được xuất mà chưa qua A/B nghe thử; "
                "không đưa vào sản xuất khi chưa đánh giá.\n", encoding="utf-8")
            evaluated = False
        (partial / "IMPORT.md").write_text(import_instructions(export_name, info, evaluated), encoding="utf-8")
        (partial / "verify_package.py").write_text(VERIFY_SCRIPT, encoding="utf-8")
        atomic_write_json(partial / "MANIFEST.json", {"export_name": export_name, "created_at": now_iso(),
                                                      "evaluated": evaluated, "files": file_manifest(partial)})
        os.replace(partial, target)
    except BaseException:
        remove_tree(partial)
        raise
    return target


def verify_package(package: Path) -> list[str]:
    """Problems with a package: changed files, and required pieces that are missing. Empty = intact."""
    package = Path(package)
    mpath = package / "MANIFEST.json"
    if not mpath.is_file():
        return ["thiếu MANIFEST.json"]
    manifest = read_json(mpath)
    problems = verify_manifest(package, manifest["files"])
    model_dir = package / "model" / manifest["export_name"]
    for needed in (model.WEIGHTS_FILE, "config.json"):
        if not (model_dir / needed).is_file():
            problems.append(f"thiếu model/{manifest['export_name']}/{needed}")
    for needed in ("PROVENANCE.json", "IMPORT.md", "EVALUATION.md"):
        if needed not in manifest["files"]:
            problems.append(f"thiếu {needed}")
    extra = sorted(p.relative_to(package).as_posix() for p in package.rglob("*")
                   if p.is_file() and p.name != "MANIFEST.json" and p.relative_to(package).as_posix() not in manifest["files"])
    problems += [f"file lạ không có trong manifest: {e}" for e in extra]
    return problems


def import_instructions(name: str, info: dict[str, Any], evaluated: bool) -> str:
    commit = info["code"]["acestep_commit"] or "ca1e85f"
    return f"""# Đưa `{name}` vào Lush Studio

Gói này chứa một thư mục model ACE-Step hoàn chỉnh (`model/{name}/`). Nó KHÔNG chứa VAE và text encoder:
máy inference phải có sẵn `checkpoints/vae` và `checkpoints/Qwen3-Embedding-0.6B` (đã có trên Render Node).

{"" if evaluated else "**Cảnh báo: gói này chưa có báo cáo đánh giá (xem EVALUATION.md).**"}

## Hai máy khác nhau

- **VPS** (website, API, database): không có GPU, không chạy model. Chép weights lên VPS không làm gì cả.
- **Render Node** (máy GPU chạy `acestep-api` và `lush-worker`): đây là nơi đặt model.

## Các bước (bạn tự làm hoặc phê duyệt từng bước; workspace không tự upload hay deploy)

1. Chép cả thư mục gói sang Render Node, ví dụ vào `<LUSH_ROOT>/data/import/{name}/`.
2. Trên Render Node, kiểm tra toàn vẹn: `python3 verify_package.py` trong thư mục gói. Phải in `OK`.
3. Xác nhận ACE-Step trên node ở commit `{commit[:12]}` (`git -C <LUSH_ROOT>/ACE-Step-1.5 rev-parse HEAD`).
   Khác commit thì dừng: model chưa được kiểm với bản đó.
4. Chép `model/{name}/` vào `<LUSH_ROOT>/ACE-Step-1.5/checkpoints/{name}/`. Không ghi đè thư mục sẵn có.
5. Smoke test trước khi cho website dùng: cho `acestep-api` nạp `{name}` và tạo một bài ngắn (30 giây), nghe thử.
   Cách chọn model `acestep-api` nạp lúc khởi động nằm trong `<LUSH_ROOT>/config/acestep.env` trên node. Worker chỉ
   đăng ký với Control Plane những DiT model mà `GET /v1/model_inventory` báo `is_loaded` (`engines/acestep.py:list_models`),
   nên model có trên đĩa mà chưa được nạp thì website không thấy.
   **Chưa kiểm chứng:** biến cấu hình chính xác để nạp một model tùy chỉnh và việc nạp hai model cùng lúc trên 12 GB VRAM.
   Kiểm tra bước này trên node trước khi đổi cấu hình production.
6. Sau khi worker đăng ký lại (tối đa ~60 giây), model xuất hiện ở Quản trị → Models với key `{name}`.
   Model mặc định của website không tự đổi; đổi mặc định là quyết định riêng của bạn.

## Rollback

Trả `acestep.env` về model cũ, khởi động lại `acestep-api` (`systemctl --user restart acestep-api lush-worker`), rồi
xóa `checkpoints/{name}/` nếu muốn. Model cũ không bị đụng tới ở bất kỳ bước nào ở trên.
"""
