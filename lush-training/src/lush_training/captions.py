"""Captions for instrumental segments: measured evidence -> draft -> human approval -> locked version.

One working file per segment (`<version>/captions/work/<segment_id>.json`) keeps structured
metadata, the evidence behind it, the caption text and every human edit apart. Unknown values stay
null; they are left out of the conditioning text instead of being guessed.

Conditioning format (read from ACE-Step ca1e85f, `training_v2/preprocess_prompt.py` and
`training/data_module.py`): per sample `caption`, `lyrics = "[Instrumental]"`, `is_instrumental`,
optional `bpm` / `keyscale` / `timesignature` (missing -> upstream writes "N/A"), `duration`.
No lyrics format or special token is invented here.
"""
from __future__ import annotations

import os
import re
from pathlib import Path
from typing import Any, Optional

from . import audio
from .dataset import load_manifest
from .util import (LushTrainingError, atomic_write_json, canonical_json, now_iso, read_json, remove_tree,
                   sha256_bytes, sha256_file)

SCHEMA = 1
INSTRUMENTAL_LYRICS = "[Instrumental]"
STATUSES = ("draft", "needs_listening", "approved")
LIST_FIELDS = ("instruments", "mood")
TEXT_FIELDS = ("genre", "subgenre", "rhythm", "harmony", "key", "time_signature", "texture", "dynamics", "structure")
NUMBER_FIELDS = ("tempo_bpm",)
METADATA_FIELDS = TEXT_FIELDS + LIST_FIELDS + NUMBER_FIELDS
EVIDENCE_SOURCES = ("tool", "human", "source_metadata")
MAX_CAPTION_WORDS = 150  # upstream truncates at 256 tokens; words are only a proxy for tokens
VOCAL_TERMS = re.compile(r"\b(vocals?|vocalist|singer|singing|sung|sings|lyrics?|rap|rapper|rapping|choir|chorus vocals|"
                         r"spoken word|voice|voices|humming|acapella|a cappella|giọng hát|ca sĩ|lời bài hát)\b",
                         re.IGNORECASE)
NEGATED_VOCAL = re.compile(r"\b(no|without|không có)\s+(vocals?|voice|singing|lyrics?|giọng hát)\b", re.IGNORECASE)
VERSION_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
TRAINING_SPLITS = ("train", "validation")


def work_path(vdir: Path, segment_id: str) -> Path:
    return Path(vdir) / "captions" / "work" / f"{segment_id}.json"


def analysis_path(vdir: Path, segment_id: str) -> Path:
    return Path(vdir) / "analysis" / f"{segment_id}.json"


# ---------- measured evidence ----------

def tool_versions() -> dict[str, Any]:
    import numpy
    import soundfile
    from . import __version__
    versions = {"lush_training": __version__, "numpy": numpy.__version__, "soundfile": soundfile.__version__}
    for name in ("librosa", "pyloudnorm"):
        try:
            versions[name] = __import__(name).__version__
        except Exception:
            versions[name] = None
    return versions


def analyze_segment(vdir: Path, segment: dict[str, Any]) -> dict[str, Any]:
    """Measure one segment. Cached by the segment's sha256: unchanged audio is never analysed twice."""
    out = analysis_path(vdir, segment["segment_id"])
    if out.is_file():
        cached = read_json(out)
        if cached.get("segment_sha256") == segment["sha256"]:
            return cached
    data, rate = audio.decode(Path(vdir) / segment["file"])
    versions = tool_versions()
    result: dict[str, Any] = {"schema": SCHEMA, "segment_id": segment["segment_id"], "segment_sha256": segment["sha256"],
                              "analyzed_at": now_iso(), "tools": versions,
                              "measured": {**audio.measure(data, rate), **audio.spectral_summary(data, rate)},
                              "estimates": {"tempo_bpm": None, "integrated_lufs": None},
                              "not_available": []}
    mono = data.mean(axis=1)
    if versions["librosa"]:
        import librosa
        tempo, _ = librosa.beat.beat_track(y=mono, sr=rate)
        result["estimates"]["tempo_bpm"] = round(float(tempo[0] if hasattr(tempo, "__len__") else tempo), 1)
    else:
        result["not_available"].append("tempo_bpm: chưa cài librosa (pip install -e .[analysis])")
    if versions["pyloudnorm"]:
        import pyloudnorm
        result["estimates"]["integrated_lufs"] = round(float(pyloudnorm.Meter(rate).integrated_loudness(data)), 2)
    else:
        result["not_available"].append("integrated_lufs: chưa cài pyloudnorm")
    result["not_available"] += ["genre, instruments, mood, harmony, structure: không có công cụ nghe hiểu audio nào được "
                                "cấu hình; các trường này do người nghe điền"]
    atomic_write_json(out, result)
    return result


# ---------- working files ----------

def empty_caption(manifest: dict[str, Any], manifest_sha: str, segment: dict[str, Any]) -> dict[str, Any]:
    return {"schema": SCHEMA, "segment_id": segment["segment_id"], "segment_sha256": segment["sha256"],
            "dataset": {**manifest["dataset"], "manifest_sha256": manifest_sha},
            "metadata": {f: None for f in METADATA_FIELDS}, "evidence": [], "caption_text": None,
            "status": "draft", "author": None, "human_edits": []}


def init_work(vdir: Path, *, analyze: bool = True) -> dict[str, int]:
    """Create a draft for every train/validation segment that has none. Existing drafts are never overwritten."""
    manifest, digest = load_manifest(vdir)
    created = kept = 0
    for seg in manifest["segments"]:
        if seg["split"] not in TRAINING_SPLITS:
            continue  # the test split gets no training caption
        path = work_path(vdir, seg["segment_id"])
        if path.is_file():
            kept += 1
            continue
        cap = empty_caption(manifest, digest, seg)
        if analyze:
            a = analyze_segment(vdir, seg)
            if a["estimates"]["tempo_bpm"] is not None:
                cap["evidence"].append({"field": "tempo_bpm", "source": "tool", "tool": "librosa.beat.beat_track",
                                        "tool_version": a["tools"]["librosa"], "value": a["estimates"]["tempo_bpm"],
                                        "note": "ước lượng tự động, có thể sai gấp đôi/một nửa; người nghe xác nhận "
                                                "trước khi đưa vào metadata"})
        atomic_write_json(path, cap)
        created += 1
    return {"created": created, "kept": kept}


def set_fields(vdir: Path, segment_id: str, *, by: str, human: bool, metadata: Optional[dict[str, Any]] = None,
               caption_text: Optional[str] = None, status: Optional[str] = None, note: str = "") -> dict[str, Any]:
    """Change a working caption and record who changed what. `human=True` only for a person's own decision."""
    path = work_path(vdir, segment_id)
    if not path.is_file():
        raise LushTrainingError(f"chưa có caption nháp cho {segment_id} (chạy `validate_captions.py init`)")
    cap = read_json(path)
    changes: list[tuple[str, Any, Any]] = []
    for field, value in (metadata or {}).items():
        if field not in METADATA_FIELDS:
            raise LushTrainingError(f"trường metadata không hợp lệ: {field}")
        changes.append((f"metadata.{field}", cap["metadata"].get(field), value))
        cap["metadata"][field] = value
    if caption_text is not None:
        changes.append(("caption_text", cap["caption_text"], caption_text))
        cap["caption_text"] = caption_text
    if status is not None:
        if status not in STATUSES:
            raise LushTrainingError(f"status phải là một trong {STATUSES}")
        if status == "approved" and not human:
            raise LushTrainingError("chỉ người dùng mới duyệt caption (approved); Claude và công cụ chỉ được đề xuất")
        changes.append(("status", cap["status"], status))
        cap["status"] = status
    cap["author"] = {"kind": "human" if human else "assistant", "name": by}
    if human:
        cap["human_edits"] += [{"at": now_iso(), "by": by, "field": f, "old": old, "new": new, "note": note}
                               for f, old, new in changes if old != new]
    atomic_write_json(path, cap)
    return cap


# ---------- validation ----------

def validate_caption(cap: Any, segment: Optional[dict[str, Any]], manifest_sha: str) -> tuple[list[str], list[str]]:
    """(errors, warnings) for one working caption."""
    errors: list[str] = []
    warnings: list[str] = []
    if not isinstance(cap, dict):
        return ["không phải JSON object"], []
    for key in ("schema", "segment_id", "segment_sha256", "dataset", "metadata", "evidence", "caption_text", "status",
                "human_edits"):
        if key not in cap:
            errors.append(f"thiếu khóa `{key}`")
    if errors:
        return errors, warnings
    if cap["schema"] != SCHEMA:
        errors.append(f"schema {cap['schema']} không được hỗ trợ")
    if segment is None:
        return errors + ["segment_id không có trong manifest của dataset version này"], warnings
    if segment["split"] not in TRAINING_SPLITS:
        errors.append("segment thuộc tập test: không được có caption train")
    if cap["segment_sha256"] != segment["sha256"]:
        errors.append("segment_sha256 không khớp audio trong manifest (caption viết cho audio khác)")
    if (cap["dataset"] or {}).get("manifest_sha256") != manifest_sha:
        errors.append("caption thuộc dataset version khác (manifest_sha256 không khớp)")
    if cap["status"] not in STATUSES:
        errors.append(f"status `{cap['status']}` không hợp lệ")
    meta = cap["metadata"]
    if not isinstance(meta, dict) or set(meta) - set(METADATA_FIELDS):
        errors.append("metadata có khóa lạ hoặc sai kiểu")
        meta = {}
    for f in TEXT_FIELDS:
        if meta.get(f) is not None and (not isinstance(meta[f], str) or not meta[f].strip()):
            errors.append(f"metadata.{f} phải là chuỗi không rỗng hoặc null")
    for f in LIST_FIELDS:
        v = meta.get(f)
        if v is not None and (not isinstance(v, list) or not v or not all(isinstance(x, str) and x.strip() for x in v)):
            errors.append(f"metadata.{f} phải là danh sách chuỗi hoặc null")
    bpm = meta.get("tempo_bpm")
    if bpm is not None and (isinstance(bpm, bool) or not isinstance(bpm, (int, float)) or not 30 <= bpm <= 300):
        errors.append("metadata.tempo_bpm phải là số trong 30-300 hoặc null")
    evidenced = set()
    for ev in cap["evidence"] if isinstance(cap["evidence"], list) else [None]:
        if not isinstance(ev, dict) or ev.get("field") not in METADATA_FIELDS or ev.get("source") not in EVIDENCE_SOURCES:
            errors.append("evidence có mục sai định dạng (cần field, source ∈ tool/human/source_metadata)")
            continue
        if ev["source"] == "tool" and not (ev.get("tool") and ev.get("tool_version")):
            errors.append(f"evidence của `{ev['field']}` thiếu tool/tool_version")
        evidenced.add(ev["field"])
    for f in METADATA_FIELDS:
        if meta.get(f) is not None and f not in evidenced:
            warnings.append(f"metadata.{f} có giá trị nhưng không có evidence")
    text = cap["caption_text"]
    if text is not None and not isinstance(text, str):
        errors.append("caption_text phải là chuỗi hoặc null")
        text = None
    for label, value in [("caption_text", text or "")] + [(f"metadata.{f}", str(meta.get(f) or "")) for f in METADATA_FIELDS]:
        stripped = NEGATED_VOCAL.sub(" ", value)
        hit = VOCAL_TERMS.search(stripped)
        if hit:
            errors.append(f"{label} nhắc tới vocal (`{hit.group(0)}`): dataset chỉ instrumental. Nếu segment có giọng, "
                          "ghi vocal review cho track thay vì mô tả giọng.")
    if text:
        words = len(text.split())
        if words > MAX_CAPTION_WORDS:
            errors.append(f"caption_text dài {words} từ (> {MAX_CAPTION_WORDS}); upstream cắt ở 256 token")
        if bpm is not None:
            mentioned = [int(x) for x in re.findall(r"(\d{2,3})\s*bpm", text, re.IGNORECASE)]
            if mentioned and all(abs(m - bpm) > 2 for m in mentioned):
                errors.append(f"caption_text ghi {mentioned} BPM nhưng metadata.tempo_bpm = {bpm}")
    if cap["status"] == "approved":
        if not text or not text.strip():
            errors.append("status approved nhưng caption_text rỗng")
        if not any(isinstance(e, dict) and e.get("field") == "status" and e.get("new") == "approved"
                   for e in cap["human_edits"]):
            errors.append("status approved nhưng không có bản ghi người duyệt trong human_edits")
    return errors, warnings


def validate_work(vdir: Path) -> dict[str, Any]:
    manifest, digest = load_manifest(vdir)
    segments = {s["segment_id"]: s for s in manifest["segments"]}
    needed = {sid for sid, s in segments.items() if s["split"] in TRAINING_SPLITS}
    work = Path(vdir) / "captions" / "work"
    report: dict[str, Any] = {"manifest_sha256": digest, "segments_needing_caption": len(needed), "errors": {},
                              "warnings": {}, "by_status": {s: [] for s in STATUSES}, "missing": []}
    seen = set()
    for path in sorted(work.glob("*.json")) if work.is_dir() else []:
        try:
            cap = read_json(path)
        except ValueError as e:
            report["errors"][path.stem] = [f"JSON hỏng: {e}"]
            continue
        sid = cap.get("segment_id") if isinstance(cap, dict) else None
        if sid != path.stem:
            report["errors"][path.stem] = ["tên file không khớp segment_id bên trong"]
            continue
        errors, warnings = validate_caption(cap, segments.get(sid), digest)
        seen.add(sid)
        if errors:
            report["errors"][sid] = errors
        if warnings:
            report["warnings"][sid] = warnings
        if not errors and cap["status"] in STATUSES:
            report["by_status"][cap["status"]].append(sid)
    report["missing"] = sorted(needed - seen)
    report["ready_to_lock"] = (not report["errors"] and not report["missing"]
                               and len(report["by_status"]["approved"]) == len(needed))
    return report


# ---------- lock + engine dataset JSON ----------

def engine_sample(cap: dict[str, Any], segment: dict[str, Any]) -> dict[str, Any]:
    """One sample in the ACE-Step dataset JSON. Unknown fields are omitted so upstream writes "N/A"."""
    meta = cap["metadata"]
    name = Path(segment["file"]).name
    sample: dict[str, Any] = {"filename": name, "audio_path": name, "caption": cap["caption_text"].strip(),
                              "genre": ", ".join(x for x in (meta.get("genre"), meta.get("subgenre")) if x),
                              "lyrics": INSTRUMENTAL_LYRICS, "is_instrumental": True, "language": "unknown",
                              "duration": int(round(segment["duration"])), "labeled": True}
    if meta.get("tempo_bpm") is not None:
        sample["bpm"] = int(round(meta["tempo_bpm"]))
    if meta.get("key"):
        sample["keyscale"] = meta["key"]
    if meta.get("time_signature"):
        sample["timesignature"] = meta["time_signature"]
    return sample


def lock(vdir: Path, caption_version: str) -> dict[str, Any]:
    """Freeze the approved captions as `<version>/captions/<caption_version>/`. Never overwrites."""
    if not VERSION_RE.match(caption_version) or caption_version == "work":
        raise LushTrainingError(f"tên caption version không hợp lệ: {caption_version!r}")
    report = validate_work(vdir)
    if not report["ready_to_lock"]:
        raise LushTrainingError(
            f"chưa khóa được caption: {len(report['errors'])} segment lỗi, {len(report['missing'])} segment thiếu, "
            f"{len(report['by_status']['draft'])} nháp, {len(report['by_status']['needs_listening'])} cần nghe lại. "
            "Mọi segment train/validation phải được người dùng duyệt.")
    manifest, digest = load_manifest(vdir)
    target = Path(vdir) / "captions" / caption_version
    if target.exists():
        raise LushTrainingError(f"caption version đã tồn tại và bất biến: {target}")
    partial = target.with_name(target.name + ".partial")
    remove_tree(partial)
    try:
        captions = []
        per_split: dict[str, list[dict[str, Any]]] = {s: [] for s in TRAINING_SPLITS}
        for seg in manifest["segments"]:
            if seg["split"] not in TRAINING_SPLITS:
                continue
            cap = read_json(work_path(vdir, seg["segment_id"]))
            captions.append(cap)
            per_split[seg["split"]].append(engine_sample(cap, seg))
        atomic_write_json(partial / "captions.json", captions)
        for split, samples in per_split.items():
            atomic_write_json(partial / f"engine.{split}.json",
                              {"metadata": {"name": f"{manifest['dataset']['name']}-{manifest['dataset']['version']}-{split}",
                                            "custom_tag": "", "tag_position": "prepend", "all_instrumental": True,
                                            "genre_ratio": 0},
                               "samples": samples})
        info = {"schema": SCHEMA, "caption_version": caption_version, "locked_at": now_iso(),
                "dataset": {**manifest["dataset"], "manifest_sha256": digest},
                "segments": {s: len(v) for s, v in per_split.items()},
                "captions_sha256": sha256_file(partial / "captions.json"),
                "conditioning": {"lyrics": INSTRUMENTAL_LYRICS, "format_source": "ACE-Step ca1e85f preprocess_prompt.py",
                                 "unknown_fields": "omitted (upstream writes N/A)"},
                "content_sha256": sha256_bytes(canonical_json(captions))}
        atomic_write_json(partial / "LOCK.json", info)
        os.replace(partial, target)
    except BaseException:
        remove_tree(partial)
        raise
    return info


def load_lock(vdir: Path, caption_version: str) -> dict[str, Any]:
    """The lock record, after checking that the dataset and the captions are still what was locked."""
    cdir = Path(vdir) / "captions" / caption_version
    if not (cdir / "LOCK.json").is_file():
        raise LushTrainingError(f"không có caption version đã khóa: {cdir}")
    info = read_json(cdir / "LOCK.json")
    _, digest = load_manifest(vdir)
    if info["dataset"]["manifest_sha256"] != digest:
        raise LushTrainingError("caption version này thuộc dataset version khác (manifest_sha256 không khớp)")
    if sha256_file(cdir / "captions.json") != info["captions_sha256"]:
        raise LushTrainingError(f"captions.json của {cdir} đã bị sửa sau khi khóa")
    return info
