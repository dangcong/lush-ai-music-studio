"""Instrumental dataset preparation: scan -> review -> immutable dataset version.

Rules enforced here:
- Original audio is only ever read.
- Train / validation / test are split by GROUP of related tracks before segmentation, so no two
  segments of the same source (or of two files judged related) end up in different splits.
- A file suspected of vocals is flagged for a human to listen to. A detector score is a screening
  signal; the decision recorded in the manifest is the human one. This workspace ships no detector.
- A dataset version is written once and never changed. Changing anything means a new version.
"""
from __future__ import annotations

import csv
import hashlib
import os
import re
from pathlib import Path
from typing import Any, Optional

import numpy as np

from . import audio
from .config import Project, load_yaml
from .util import (LushTrainingError, atomic_write_json, canonical_json, now_iso, read_json, remove_tree,
                   sha256_bytes, sha256_file)

SCHEMA = 1
SPLITS = ("train", "validation", "test")
VERSION_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$")
DEFAULTS: dict[str, dict[str, Any]] = {
    "segment": {"seconds": 60.0, "min_seconds": 30.0, "max_silence_ratio": 0.5},
    "split": {"validation": 0.1, "test": 0.1, "seed": 1234},
    "checks": {"min_duration": 30.0, "min_sample_rate": 32000, "max_clipped_ratio": 0.001,
               "max_silence_ratio": 0.5, "near_duplicate_similarity": 0.97},
}
VERSION_WORDS = re.compile(r"\b(remix|edit|version|ver|master|mastered|remaster(ed)?|final|mix|alt|alternate|"
                           r"demo|instrumental|inst|extended|short|loop|take|v\d+)\b")


def load_dataset_config(path: Path) -> dict[str, Any]:
    cfg = load_yaml(path)
    name = str(cfg.get("name") or "")
    if not VERSION_RE.match(name):
        raise LushTrainingError(f"{path}: `name` phải khớp {VERSION_RE.pattern}")
    dirs = cfg.get("source_dirs")
    if not isinstance(dirs, list) or not dirs:
        raise LushTrainingError(f"{path}: `source_dirs` phải là danh sách thư mục nhạc")
    for section, defaults in DEFAULTS.items():
        merged = dict(defaults)
        merged.update(cfg.get(section) or {})
        unknown = set(merged) - set(defaults)
        if unknown:
            raise LushTrainingError(f"{path}: khóa không hợp lệ trong `{section}`: {sorted(unknown)}")
        cfg[section] = merged
    split = cfg["split"]
    if not (0 <= split["validation"] < 1 and 0 <= split["test"] < 1 and split["validation"] + split["test"] < 1):
        raise LushTrainingError(f"{path}: tỉ lệ split không hợp lệ")
    if cfg["segment"]["min_seconds"] > cfg["segment"]["seconds"]:
        raise LushTrainingError(f"{path}: segment.min_seconds lớn hơn segment.seconds")
    cfg.setdefault("rights", {})
    cfg.setdefault("vocal_flag_threshold", 0.5)
    return cfg


def work_dir(project: Project, name: str) -> Path:
    return project.dir("datasets") / name / "_work"


def version_dir(project: Project, name: str, version: str) -> Path:
    if not VERSION_RE.match(version):
        raise LushTrainingError(f"tên version không hợp lệ: {version!r}")
    return project.dir("datasets") / name / version


# ---------- scan ----------

def find_audio(source_dirs: list[str]) -> list[tuple[Path, str]]:
    """(absolute path, path relative to its source dir) for every audio file, recursively, sorted."""
    found = []
    for d in source_dirs:
        root = Path(d).expanduser()
        if not root.is_dir():
            raise LushTrainingError(f"không có thư mục nhạc: {root}")
        for p in sorted(root.rglob("*")):
            if p.is_file() and p.suffix.lower() in audio.AUDIO_EXTENSIONS:
                found.append((p.resolve(), p.relative_to(root).as_posix()))
    return found


def _cache_key(path: Path) -> str:
    st = path.stat()
    return sha256_bytes(f"{path}|{st.st_size}|{st.st_mtime_ns}".encode("utf-8"))


def scan_file(path: Path, cache: Path) -> dict[str, Any]:
    """Hash, decode and measure one file. Cached by (path, size, mtime) so unchanged files are not redone."""
    key = _cache_key(path)
    meta_path, fp_path = cache / f"{key}.json", cache / f"{key}.npy"
    if meta_path.is_file() and fp_path.is_file():
        entry = read_json(meta_path)
        entry["_fp"] = np.load(fp_path)
        return entry
    entry: dict[str, Any] = {"sha256": sha256_file(path), "bytes": path.stat().st_size, "decodable": True,
                             "metrics": None, "error": None}
    fp = np.zeros((0, audio.FP_BANDS), dtype=np.float32)
    try:
        data, rate = audio.decode(path)
        entry["metrics"] = audio.measure(data, rate)
        fp = audio.fingerprint(data, rate)
    except LushTrainingError as e:
        entry["decodable"] = False
        entry["error"] = str(e)
    cache.mkdir(parents=True, exist_ok=True)
    np.save(fp_path, fp)
    atomic_write_json(meta_path, entry)
    entry["_fp"] = fp
    return entry


def normalize_title(rel: str) -> str:
    stem = Path(rel).stem.lower()
    stem = re.sub(r"[\(\[\{].*?[\)\]\}]", " ", stem)
    stem = VERSION_WORDS.sub(" ", stem)
    return re.sub(r"[^a-z0-9]+", " ", stem).strip() or Path(rel).stem.lower()


def read_csv_by_path(path: Optional[str]) -> dict[str, dict[str, str]]:
    if not path:
        return {}
    p = Path(path).expanduser()
    if not p.is_file():
        raise LushTrainingError(f"không có file CSV: {p}")
    with open(p, newline="", encoding="utf-8-sig") as f:
        rows = list(csv.DictReader(f))
    if rows and "path" not in rows[0]:
        raise LushTrainingError(f"{p}: cần cột `path`")
    return {r["path"].replace("\\", "/").strip(): r for r in rows}


class _Groups:
    def __init__(self) -> None:
        self.parent: dict[str, str] = {}

    def find(self, x: str) -> str:
        self.parent.setdefault(x, x)
        while self.parent[x] != x:
            self.parent[x] = self.parent[self.parent[x]]
            x = self.parent[x]
        return x

    def union(self, a: str, b: str) -> None:
        ra, rb = self.find(a), self.find(b)
        if ra != rb:
            self.parent[max(ra, rb)] = min(ra, rb)


def scan(cfg: dict[str, Any], project: Project) -> dict[str, Any]:
    """Scan the source folders. Writes `<datasets>/<name>/_work/scan.json` and returns it."""
    checks = cfg["checks"]
    meta_csv = read_csv_by_path(cfg.get("metadata_csv"))
    screen_csv = read_csv_by_path(cfg.get("vocal_screening_csv"))
    cache = project.dir("cache") / "scan"
    files = find_audio(cfg["source_dirs"])
    if not files:
        raise LushTrainingError("không tìm thấy file WAV/FLAC/MP3 nào trong source_dirs")
    tracks: list[dict[str, Any]] = []
    fps: dict[str, np.ndarray] = {}
    first_by_sha: dict[str, str] = {}
    for path, rel in files:
        e = scan_file(path, cache)
        fp = e.pop("_fp")
        row = meta_csv.get(rel) or meta_csv.get(Path(rel).name) or {}
        screen = screen_csv.get(rel) or screen_csv.get(Path(rel).name)
        t: dict[str, Any] = {"track_id": e["sha256"][:16], "sha256": e["sha256"], "source_path": str(path), "rel": rel,
                             "bytes": e["bytes"], "metrics": e["metrics"], "issues": [], "warnings": [],
                             "exclude_reason": None, "work_group": (row.get("work_group") or "").strip() or None,
                             "rights_note": (row.get("rights_note") or "").strip() or None,
                             "vocal_screen": None}
        if screen:
            try:
                t["vocal_screen"] = {"score": float(screen["score"]), "tool": screen.get("tool") or "unknown",
                                     "tool_version": screen.get("tool_version") or "unknown"}
            except (KeyError, ValueError):
                t["warnings"].append("dòng vocal_screening_csv không đọc được")
        if not e["decodable"]:
            t["exclude_reason"] = f"không giải mã được: {e['error']}"
        elif (row.get("exclude") or "").strip().lower() in ("1", "true", "yes", "x"):
            t["exclude_reason"] = "người dùng loại trong metadata_csv"
        elif e["sha256"] in first_by_sha:
            t["exclude_reason"] = f"trùng từng byte với {first_by_sha[e['sha256']]}"
        else:
            m = e["metrics"]
            if m["duration"] < checks["min_duration"]:
                t["exclude_reason"] = f"ngắn hơn {checks['min_duration']} giây ({m['duration']} giây)"
            if m["sample_rate"] < checks["min_sample_rate"]:
                t["warnings"].append(f"sample rate thấp: {m['sample_rate']} Hz")
            if m["channels"] == 1:
                t["warnings"].append("mono")
            if m["clipped_sample_ratio"] > checks["max_clipped_ratio"]:
                t["warnings"].append(f"clipping: {m['clipped_sample_ratio']:.4%} mẫu")
            if m["silence_ratio"] > checks["max_silence_ratio"]:
                t["warnings"].append(f"im lặng {m['silence_ratio']:.0%} thời lượng")
        if e["decodable"] and e["sha256"] not in first_by_sha:
            first_by_sha[e["sha256"]] = rel
            fps[t["track_id"]] = fp
        tracks.append(t)

    # Related tracks: same declared/guessed work, or near-duplicate audio.
    groups = _Groups()
    by_title: dict[str, str] = {}
    usable = [t for t in tracks if t["exclude_reason"] is None]
    for t in usable:
        key = "csv:" + t["work_group"] if t["work_group"] else "title:" + normalize_title(t["rel"])
        groups.union(t["track_id"], by_title.setdefault(key, t["track_id"]))
    near: list[dict[str, Any]] = []
    for i, a in enumerate(usable):
        for b in usable[i + 1:]:
            da, db = a["metrics"]["duration"], b["metrics"]["duration"]
            if abs(da - db) > 0.02 * max(da, db) + 1.0:
                continue
            sim = audio.fingerprint_similarity(fps[a["track_id"]], fps[b["track_id"]])
            if sim >= checks["near_duplicate_similarity"]:
                near.append({"a": a["rel"], "b": b["rel"], "similarity": round(sim, 4)})
                groups.union(a["track_id"], b["track_id"])
    for t in usable:
        t["group"] = groups.find(t["track_id"])

    result = {
        "schema": SCHEMA, "dataset": cfg["name"], "scanned_at": now_iso(),
        "config_sha256": sha256_bytes(canonical_json({k: cfg[k] for k in ("segment", "split", "checks")})),
        "files_found": len(tracks),
        "usable_tracks": len(usable),
        "usable_hours": round(sum(t["metrics"]["duration"] for t in usable) / 3600, 3),
        "groups": len({t["group"] for t in usable}),
        "near_duplicates": near,
        "vocal_screening": "csv" if screen_csv else "none",
        "tracks": tracks,
    }
    atomic_write_json(work_dir(project, cfg["name"]) / "scan.json", result)
    return result


# ---------- vocal review ----------

VOCAL_DECISIONS = ("instrumental", "vocal", "unsure", "excluded")  # excluded: reviewer drops the track for another reason


def load_reviews(project: Project, name: str) -> dict[str, dict[str, Any]]:
    p = work_dir(project, name) / "vocal_review.json"
    return read_json(p) if p.is_file() else {}


def record_review(project: Project, name: str, sha256: str, decision: str, by: str, note: str = "") -> None:
    """Record what a HUMAN decided after listening. `by` is the person's name, never a tool."""
    if decision not in VOCAL_DECISIONS:
        raise LushTrainingError(f"quyết định phải là một trong {VOCAL_DECISIONS}")
    if not by.strip():
        raise LushTrainingError("cần tên người đã nghe")
    reviews = load_reviews(project, name)
    reviews[sha256] = {"decision": decision, "by": by.strip(), "at": now_iso(), "note": note}
    atomic_write_json(work_dir(project, name) / "vocal_review.json", reviews)


EXCLUDING_DECISIONS = ("vocal", "excluded")


def excluded_tracks(vdir: Path) -> dict[str, dict[str, Any]]:
    """sha256 -> review, for tracks a person excluded (voice or other reason). Reads the dataset's live
    review file, so a decision made after a version was built is still seen by lock/export of that version."""
    p = Path(vdir).parent / "_work" / "vocal_review.json"
    reviews = read_json(p) if p.is_file() else {}
    return {sha: r for sha, r in reviews.items() if r.get("decision") in EXCLUDING_DECISIONS}


def vocal_status(track: dict[str, Any], reviews: dict[str, dict[str, Any]], threshold: float) -> dict[str, Any]:
    review = reviews.get(track["sha256"])
    screen = track.get("vocal_screen")
    flagged = bool(screen and screen["score"] >= threshold)
    if review:
        state = {"instrumental": "confirmed_instrumental", "vocal": "has_vocal", "unsure": "needs_listening",
                 "excluded": "excluded_by_reviewer"}[review["decision"]]
    elif flagged:
        state = "needs_listening"
    elif screen:
        state = "screened_not_flagged"
    else:
        state = "not_screened"
    return {"state": state, "screen": screen, "review": review}


# ---------- build ----------

def assign_splits(tracks: list[dict[str, Any]], split: dict[str, Any]) -> dict[str, str]:
    """group -> split. Deterministic for a given seed; whole groups move together."""
    durations: dict[str, float] = {}
    for t in tracks:
        durations[t["group"]] = durations.get(t["group"], 0.0) + t["metrics"]["duration"]
    order = sorted(durations, key=lambda g: hashlib.sha256(f"{split['seed']}|{g}".encode()).hexdigest())
    total = sum(durations.values())
    wanted = {"test": split["test"] * total, "validation": split["validation"] * total}
    needed = 1 + sum(1 for k in ("test", "validation") if split[k] > 0)
    if len(order) < needed:
        raise LushTrainingError(f"chỉ có {len(order)} nhóm tác phẩm, không đủ để chia {needed} tập mà không rò rỉ. "
                                "Thêm dữ liệu hoặc đặt validation/test = 0.")
    result: dict[str, str] = {}
    have = {"test": 0.0, "validation": 0.0}
    remaining = len(order)
    for g in order:
        target = "train"
        for name in ("test", "validation"):
            others_empty = sum(1 for k in ("test", "validation") if k != name and split[k] > 0 and have[k] == 0)
            if split[name] > 0 and have[name] < wanted[name] and remaining > 1 + others_empty:
                target = name
                break
        if target != "train":
            have[target] += durations[g]
        result[g] = target
        remaining -= 1
    if "train" not in result.values():
        raise LushTrainingError("split không để lại nhóm nào cho train")
    return result


def plan_segments(duration: float, seg: dict[str, Any]) -> list[tuple[float, float]]:
    """Non-overlapping windows; a trailing remainder is kept only if it reaches min_seconds."""
    out, start = [], 0.0
    while duration - start >= seg["seconds"]:
        out.append((start, start + seg["seconds"]))
        start += seg["seconds"]
    if duration - start >= seg["min_seconds"]:
        out.append((start, duration))
    return out


def build(cfg: dict[str, Any], project: Project, version: str, previous: Optional[str] = None) -> dict[str, Any]:
    """Lock a dataset version: split by group, cut segments, write an immutable manifest.
    With `previous`, every track that was in that version keeps its split (only new groups are assigned),
    so excluding tracks never moves the others between train and validation."""
    name = cfg["name"]
    target = version_dir(project, name, version)
    if target.exists():
        raise LushTrainingError(f"dataset version đã tồn tại và bất biến: {target}. Dùng tên version mới.")
    scan_path = work_dir(project, name) / "scan.json"
    if not scan_path.is_file():
        raise LushTrainingError("chưa có kết quả scan. Chạy `prepare_dataset.py scan` trước.")
    scanned = read_json(scan_path)
    if scanned["config_sha256"] != sha256_bytes(canonical_json({k: cfg[k] for k in ("segment", "split", "checks")})):
        raise LushTrainingError("cấu hình dataset đã đổi sau lần scan. Chạy scan lại.")
    rights = cfg.get("rights") or {}
    if rights.get("confirmed") is not True or not str(rights.get("confirmed_by") or "").strip():
        raise LushTrainingError("chưa xác nhận quyền sử dụng: đặt rights.confirmed: true và rights.confirmed_by "
                                "trong cấu hình dataset. Workspace không tự xác nhận thay bạn.")
    reviews = load_reviews(project, name)
    usable, excluded, blocked = [], [], []
    for t in scanned["tracks"]:
        if t["exclude_reason"]:
            excluded.append({"source_path": t["source_path"], "sha256": t["sha256"], "reason": t["exclude_reason"]})
            continue
        if not Path(t["source_path"]).is_file() or sha256_file(Path(t["source_path"])) != t["sha256"]:
            raise LushTrainingError(f"file đã đổi hoặc mất sau khi scan: {t['source_path']}. Chạy scan lại.")
        v = vocal_status(t, reviews, cfg["vocal_flag_threshold"])
        if v["state"] == "has_vocal":
            excluded.append({"source_path": t["source_path"], "sha256": t["sha256"], "reason": "người nghe xác nhận có vocal"})
        elif v["state"] == "excluded_by_reviewer":
            note = (v["review"] or {}).get("note") or ""
            excluded.append({"source_path": t["source_path"], "sha256": t["sha256"],
                             "reason": "người duyệt loại" + (f": {note}" if note else "")})
        elif v["state"] == "needs_listening":
            blocked.append(t["rel"])
        else:
            usable.append({**t, "vocal": v})
    if blocked:
        raise LushTrainingError(f"{len(blocked)} file nghi có vocal chưa được người nghe quyết định "
                                f"(ghi bằng `prepare_dataset.py review`): {blocked[:10]}")
    if not usable:
        raise LushTrainingError("không còn track nào dùng được")
    pinned: dict[str, str] = {}
    if previous:
        prev_manifest, _ = load_manifest(version_dir(project, name, previous))
        prev_split = {t["track_id"]: t["split"] for t in prev_manifest["tracks"]}
        for t in usable:
            if t["track_id"] in prev_split:
                pinned.setdefault(t["group"], prev_split[t["track_id"]])
    fresh = [t for t in usable if t["group"] not in pinned]
    split_of = dict(pinned)
    if fresh:
        try:
            split_of.update(assign_splits(fresh, cfg["split"]))
        except LushTrainingError:
            if not pinned:
                raise
            split_of.update({t["group"]: "train" for t in fresh})

    partial = target.with_name(target.name + ".partial")
    remove_tree(partial)
    segments: list[dict[str, Any]] = []
    tracks_out: list[dict[str, Any]] = []
    try:
        for t in usable:
            split = split_of[t["group"]]
            data, rate = audio.decode(Path(t["source_path"]))
            count = 0
            for start, end in plan_segments(len(data) / rate, cfg["segment"]):
                chunk = data[int(start * rate): int(end * rate)]
                m = audio.measure(chunk, rate)
                if m["silence_ratio"] > cfg["segment"]["max_silence_ratio"]:
                    continue
                seg_id = f"{t['track_id']}-{int(round(start * 1000)):08d}"
                rel = f"segments/{split}/{seg_id}.wav"
                audio.write_segment(partial / rel, chunk, rate)
                segments.append({"segment_id": seg_id, "track_id": t["track_id"], "split": split,
                                 "start": round(start, 3), "end": round(end, 3), "duration": m["duration"],
                                 "file": rel, "sha256": sha256_file(partial / rel)})
                count += 1
            tracks_out.append({"track_id": t["track_id"], "sha256": t["sha256"], "source_path": t["source_path"],
                               "group": t["group"], "split": split, "segments": count, "metrics": t["metrics"],
                               "warnings": t["warnings"], "vocal": t["vocal"], "rights_note": t["rights_note"]})
        # Leakage guard: computed from the output, independent of how splits were assigned.
        seen: dict[str, str] = {}
        for t in tracks_out:
            if seen.setdefault(t["group"], t["split"]) != t["split"]:
                raise LushTrainingError(f"rò rỉ split: nhóm {t['group']} nằm ở hai tập")
        if not any(s["split"] == "train" for s in segments):
            raise LushTrainingError("không có segment train nào")

        def totals(split: str) -> dict[str, Any]:
            segs = [s for s in segments if s["split"] == split]
            return {"tracks": sum(1 for t in tracks_out if t["split"] == split),
                    "groups": len({t["group"] for t in tracks_out if t["split"] == split}),
                    "segments": len(segs), "hours": round(sum(s["duration"] for s in segs) / 3600, 4)}
        not_screened = sum(1 for t in tracks_out if t["vocal"]["state"] == "not_screened")
        manifest = {
            "schema": SCHEMA, "dataset": {"name": name, "version": version}, "created_at": now_iso(),
            "instrumental_only": True,
            "config": {k: cfg[k] for k in ("segment", "split", "checks")},
            "rights": rights,
            "vocal_screening": {
                "method": scanned["vocal_screening"], "tracks_not_screened": not_screened,
                "tracks_confirmed_by_listening": sum(1 for t in tracks_out if t["vocal"]["state"] == "confirmed_instrumental"),
                "note": "Detector (nếu có) chỉ là tín hiệu sàng lọc. Track `not_screened` chưa qua detector lẫn người nghe.",
            },
            "totals": {s: totals(s) for s in SPLITS},
            "near_duplicates": scanned["near_duplicates"],
            "tracks": tracks_out, "excluded": excluded, "segments": segments,
        }
        atomic_write_json(partial / "manifest.json", manifest)
        (partial / "manifest.sha256").write_text(sha256_file(partial / "manifest.json") + "\n", encoding="utf-8")
        os.replace(partial, target)
    except BaseException:
        remove_tree(partial)
        raise
    return manifest


def load_manifest(vdir: Path, *, verify_files: bool = False) -> tuple[dict[str, Any], str]:
    """(manifest, sha256 of manifest.json). Fails if the manifest was edited after it was locked."""
    mpath = Path(vdir) / "manifest.json"
    if not mpath.is_file():
        raise LushTrainingError(f"không có dataset version tại {vdir}")
    digest = sha256_file(mpath)
    recorded = (Path(vdir) / "manifest.sha256").read_text(encoding="utf-8").strip()
    if digest != recorded:
        raise LushTrainingError(f"manifest của {vdir} đã bị sửa sau khi khóa (sha256 không khớp)")
    manifest = read_json(mpath)
    if verify_files:
        for s in manifest["segments"]:
            p = Path(vdir) / s["file"]
            if not p.is_file() or sha256_file(p) != s["sha256"]:
                raise LushTrainingError(f"segment hỏng hoặc mất: {p}")
    return manifest, digest


def scan_summary(scanned: dict[str, Any], reviews: dict[str, dict[str, Any]], threshold: float) -> str:
    lines = [f"Dataset `{scanned['dataset']}`: {scanned['files_found']} file, {scanned['usable_tracks']} track dùng được, "
             f"{scanned['usable_hours']} giờ, {scanned['groups']} nhóm tác phẩm."]
    excluded = [t for t in scanned["tracks"] if t["exclude_reason"]]
    if excluded:
        lines.append(f"Loại {len(excluded)} file:")
        lines += [f"  - {t['rel']}: {t['exclude_reason']}" for t in excluded[:30]]
    warned = [t for t in scanned["tracks"] if not t["exclude_reason"] and t["warnings"]]
    if warned:
        lines.append(f"Cảnh báo kỹ thuật ở {len(warned)} file (vẫn dùng, trừ khi bạn loại qua metadata_csv):")
        lines += [f"  - {t['rel']}: {'; '.join(t['warnings'])}" for t in warned[:30]]
    if scanned["near_duplicates"]:
        lines.append(f"{len(scanned['near_duplicates'])} cặp nghi gần trùng (được xếp cùng nhóm, không tự loại):")
        lines += [f"  - {n['a']} ~ {n['b']} ({n['similarity']})" for n in scanned["near_duplicates"][:30]]
    states: dict[str, list[str]] = {}
    for t in scanned["tracks"]:
        if not t["exclude_reason"]:
            states.setdefault(vocal_status(t, reviews, threshold)["state"], []).append(t["rel"])
    lines.append("Vocal: " + ", ".join(f"{k}: {len(v)}" for k, v in sorted(states.items())))
    if states.get("needs_listening"):
        lines.append("Cần bạn nghe lại: " + ", ".join(states["needs_listening"][:30]))
    if scanned["vocal_screening"] == "none":
        lines.append("Chưa có detector vocal nào chạy trên bộ này. Workspace không kết luận dataset sạch vocal.")
    return "\n".join(lines)
