"""Evaluation of a candidate checkpoint against its parent: plan, technical checks, blind A/B sheet,
memorization screening, report.

What this module does NOT do: render audio (that needs the inference engine on a GPU; the plan file
says exactly what to render), judge musical quality, or detect vocals. Loss and the numbers below are
screening signals. The decision to keep a checkpoint is the listener's.
"""
from __future__ import annotations

import csv
import random
import shutil
from pathlib import Path
from typing import Any, Optional

from . import audio
from .config import load_yaml
from .dataset import load_manifest
from .util import LushTrainingError, atomic_write_json, now_iso, read_json

SIDES = ("parent", "candidate")
CRITERIA = ("melody", "harmony", "rhythm", "arrangement", "timbre", "structure", "repetition_diversity",
            "prompt_style_adherence", "audio_fidelity", "vocal_heard")
MEMORIZATION_THRESHOLD = 0.9
MIN_OVERLAP_ROWS = 8  # 4 seconds of fingerprint


def make_plan(prompts_file: Path, out_dir: Path, *, run_id: str, checkpoint_name: str) -> dict[str, Any]:
    """Write `plan.json`: every (prompt, seed) both sides must render, with the exact file names."""
    doc = load_yaml(prompts_file)
    prompts, seeds = doc.get("prompts"), doc.get("seeds")
    if not doc.get("version") or not isinstance(prompts, list) or not prompts or not isinstance(seeds, list) or len(seeds) < 2:
        raise LushTrainingError(f"{prompts_file}: cần `version`, danh sách `prompts` và ít nhất 2 `seeds`")
    items = []
    for p in prompts:
        if not p.get("id") or not p.get("prompt") or not p.get("duration"):
            raise LushTrainingError(f"{prompts_file}: mỗi prompt cần id, prompt, duration")
        for seed in seeds:
            items.append({"id": f"{p['id']}__s{seed}", "prompt_id": p["id"], "prompt": p["prompt"], "seed": int(seed),
                          "duration": float(p["duration"]), "in_style": bool(p.get("in_style", True)),
                          "lyrics": "[Instrumental]", "file": f"{p['id']}__s{seed}.wav"})
    plan = {"created_at": now_iso(), "run_id": run_id, "checkpoint": checkpoint_name,
            "prompt_set_version": str(doc["version"]), "instrumental_only": True,
            "render": "Render mỗi mục hai lần với CÙNG seed, steps, guidance: một lần bằng model gốc vào `parent/`, một lần "
                      "bằng model ứng viên vào `candidate/`. Ghi steps/guidance đã dùng vào `render_settings.json`.",
            "items": items}
    for side in SIDES:
        (Path(out_dir) / side).mkdir(parents=True, exist_ok=True)
    atomic_write_json(Path(out_dir) / "plan.json", plan)
    return plan


def check_side(eval_dir: Path, side: str, plan: dict[str, Any]) -> list[dict[str, Any]]:
    rows = []
    for item in plan["items"]:
        path = Path(eval_dir) / side / item["file"]
        row: dict[str, Any] = {"id": item["id"], "side": side, "file": item["file"], "problems": []}
        if not path.is_file():
            row["problems"].append("chưa render")
        else:
            try:
                data, rate = audio.decode(path)
                m = audio.measure(data, rate)
                row["metrics"] = m
                if abs(m["duration"] - item["duration"]) > max(2.0, 0.05 * item["duration"]):
                    row["problems"].append(f"thời lượng {m['duration']} s, yêu cầu {item['duration']} s")
                if m["silence_ratio"] > 0.5:
                    row["problems"].append(f"im lặng {m['silence_ratio']:.0%}")
                if m["clipped_sample_ratio"] > 0.001:
                    row["problems"].append(f"clipping {m['clipped_sample_ratio']:.3%}")
                if m["rms_dbfs"] < -45:
                    row["problems"].append(f"quá nhỏ ({m['rms_dbfs']} dBFS RMS)")
            except LushTrainingError as e:
                row["problems"].append(str(e))
        rows.append(row)
    return rows


def make_ab(eval_dir: Path, *, seed: int = 0) -> dict[str, Any]:
    """Blind A/B: copies each rendered pair under neutral names in random order. The key is a separate file."""
    eval_dir = Path(eval_dir)
    plan = read_json(eval_dir / "plan.json")
    ab = eval_dir / "ab"
    if ab.exists():
        raise LushTrainingError(f"{ab} đã tồn tại; xóa tay nếu muốn tạo lại (thứ tự A/B sẽ khác)")
    rng = random.Random(seed)
    pairs = [i for i in plan["items"] if all((eval_dir / s / i["file"]).is_file() for s in SIDES)]
    if not pairs:
        raise LushTrainingError("chưa có cặp nào được render đủ hai phía")
    rng.shuffle(pairs)
    ab.mkdir()
    key = {}
    with open(ab / "sheet.csv", "w", newline="", encoding="utf-8-sig") as f:
        w = csv.writer(f)
        w.writerow(["pair", "prompt", "listener", "preferred (A/B/same)", *[f"{c} (A/B/same)" for c in CRITERIA[:-1]],
                    "vocal_heard (A/B/both/none)", "notes"])
        for n, item in enumerate(pairs, 1):
            sides = list(SIDES)
            rng.shuffle(sides)
            for label, side in zip("AB", sides):
                shutil.copy2(eval_dir / side / item["file"], ab / f"pair-{n:03d}-{label}.wav")
            key[f"pair-{n:03d}"] = {"A": sides[0], "B": sides[1], "id": item["id"], "in_style": item["in_style"]}
            w.writerow([f"pair-{n:03d}", item["prompt"]] + [""] * (len(CRITERIA) + 3))
    atomic_write_json(eval_dir / "ab_key.json", {"seed": seed, "pairs": key,
                                                 "note": "Không mở file này trước khi nghe xong."})
    return {"pairs": len(pairs), "skipped": len(plan["items"]) - len(pairs)}


def sliding_similarity(short: Any, long: Any) -> float:
    if len(short) > len(long):
        short, long = long, short
    if len(short) < MIN_OVERLAP_ROWS:
        return 0.0
    best = 0.0
    for offset in range(0, len(long) - len(short) + 1):
        best = max(best, audio.fingerprint_similarity(short, long[offset: offset + len(short)]))
    return best


def memorization_screen(eval_dir: Path, dataset_version_dir: Path, *, threshold: float = MEMORIZATION_THRESHOLD) -> dict[str, Any]:
    """Coarse similarity of each candidate render to every TRAIN segment (and, as a reference level, to
    validation segments the model never saw). Flags only near-verbatim reproduction; a melody replayed in
    another key, tempo or arrangement is not detected. Flagged pairs must be listened to."""
    eval_dir = Path(eval_dir)
    plan = read_json(eval_dir / "plan.json")
    manifest, _ = load_manifest(dataset_version_dir)
    fps: dict[str, list[tuple[str, Any]]] = {"train": [], "validation": []}
    for seg in manifest["segments"]:
        if seg["split"] in fps:
            data, rate = audio.decode(Path(dataset_version_dir) / seg["file"])
            fps[seg["split"]].append((seg["segment_id"], audio.fingerprint(data, rate)))
    rows = []
    for item in plan["items"]:
        path = eval_dir / "candidate" / item["file"]
        if not path.is_file():
            continue
        data, rate = audio.decode(path)
        fp = audio.fingerprint(data, rate)
        best = {split: max(((sliding_similarity(fp, sfp), sid) for sid, sfp in entries), default=(0.0, None))
                for split, entries in fps.items()}
        rows.append({"id": item["id"], "max_train_similarity": round(best["train"][0], 4), "train_segment": best["train"][1],
                     "max_validation_similarity": round(best["validation"][0], 4),
                     "flagged": best["train"][0] >= threshold})
    return {"threshold": threshold, "method": "log band-energy envelope, cosine, sliding offset (xem audio.fingerprint)",
            "rows": rows, "flagged": [r["id"] for r in rows if r["flagged"]]}


def write_report(eval_dir: Path, dataset_version_dir: Optional[Path] = None) -> Path:
    eval_dir = Path(eval_dir)
    plan = read_json(eval_dir / "plan.json")
    checks = {side: check_side(eval_dir, side, plan) for side in SIDES}
    lines = [f"# Evaluation — run `{plan['run_id']}`, checkpoint `{plan['checkpoint']}`", "",
             f"Tạo lúc {now_iso()}. Bộ prompt instrumental version `{plan['prompt_set_version']}`, "
             f"{len(plan['items'])} mục (prompt × seed).", "",
             "Các số dưới đây là tín hiệu sàng lọc. Chúng không chứng minh nhạc hay hơn; quyết định thuộc về người nghe.", "",
             "## Kiểm tra kỹ thuật", "", "| Phía | Đã render | Có vấn đề |", "|---|---|---|"]
    for side in SIDES:
        rendered = sum(1 for r in checks[side] if "chưa render" not in r["problems"])
        bad = [r for r in checks[side] if r["problems"] and "chưa render" not in r["problems"]]
        lines.append(f"| {side} | {rendered}/{len(checks[side])} | {len(bad)} |")
    for side in SIDES:
        for r in checks[side]:
            if r["problems"]:
                lines.append(f"- `{side}/{r['file']}`: {'; '.join(r['problems'])}")
    lines += ["", "## Memorization (so với segment train)", ""]
    if dataset_version_dir is None:
        lines.append("Chưa chạy (cần đường dẫn dataset version).")
        memo = None
    else:
        memo = memorization_screen(eval_dir, dataset_version_dir)
        lines.append(f"Ngưỡng {memo['threshold']}; phương pháp: {memo['method']}. Chỉ bắt được trường hợp lặp lại gần nguyên văn.")
        lines.append(f"Bị gắn cờ, cần nghe lại: {', '.join(memo['flagged']) if memo['flagged'] else 'không có'}.")
        if memo["rows"]:
            lines.append(f"Độ giống cao nhất với train: {max(r['max_train_similarity'] for r in memo['rows'])}; "
                         f"với validation (mức tham chiếu): {max(r['max_validation_similarity'] for r in memo['rows'])}.")
    lines += ["", "## Vocal contamination", "",
              "Không có detector nào được cấu hình. Người nghe ghi cột `vocal_heard` trong `ab/sheet.csv` cho từng cặp.", "",
              "## Nghe A/B mù", ""]
    sheet = eval_dir / "ab" / "sheet.csv"
    if not sheet.is_file():
        lines.append("Chưa tạo phiếu nghe (`evaluate.py ab`).")
    else:
        key = read_json(eval_dir / "ab_key.json")["pairs"]
        with open(sheet, newline="", encoding="utf-8-sig") as f:
            rows = [r for r in csv.DictReader(f) if (r.get("preferred (A/B/same)") or "").strip()]
        if not rows:
            lines.append(f"Phiếu `ab/sheet.csv` có {len(key)} cặp, chưa ai điền. Chưa có kết luận nào về chất lượng.")
        else:
            tally = {"candidate": 0, "parent": 0, "same": 0}
            vocal = 0
            for r in rows:
                choice = r["preferred (A/B/same)"].strip().upper()
                pair = key.get(r["pair"])
                if pair and choice in ("A", "B"):
                    tally[pair[choice]] += 1
                elif choice == "SAME":
                    tally["same"] += 1
                heard = (r.get("vocal_heard (A/B/both/none)") or "").strip().upper()
                if pair and (heard == "BOTH" or (heard in ("A", "B") and pair[heard] == "candidate")):
                    vocal += 1
            listeners = sorted({r.get("listener", "").strip() for r in rows if r.get("listener", "").strip()})
            lines.append(f"{len(rows)} lượt chấm của {len(listeners) or 'chưa ghi tên'} người nghe: thích ứng viên {tally['candidate']}, "
                         f"thích model gốc {tally['parent']}, như nhau {tally['same']}. "
                         f"Nghe thấy vocal ở ứng viên: {vocal} lượt.")
            lines.append("Với ít người nghe và ít cặp, kết quả chỉ đủ để thấy khác biệt lớn.")
    lines += ["", "## Quyết định", "", "Chưa có. Người dùng chọn giữ hay bỏ checkpoint này sau khi nghe."]
    atomic_write_json(eval_dir / "report.json", {"created_at": now_iso(), "checks": checks, "memorization": memo})
    out = eval_dir / "EVALUATION.md"
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return out
