"""Pilot report from the review log: how good were the machine drafts, and what did review cost.

The last decision per segment counts; review time adds up across visits to the same segment.
Numbers describe this reviewer on this batch only; they are not a quality score for the music.
"""
from __future__ import annotations

import statistics
from pathlib import Path
from typing import Any, Optional

from . import captions
from .dataset import load_manifest
from .util import now_iso, read_json, read_jsonl

HALLUCINATION_LABELS = {"instrument": "Bịa nhạc cụ", "mood": "Bịa mood", "genre": "Sai genre",
                        "tempo": "Sai BPM", "key": "Sai key", "other": "Khác"}


def _agreement(tool: Any, human: dict[str, Any], field: str, same) -> Optional[str]:
    """The review page never pre-fills machine estimates, so an empty field means "not filled",
    not "rejected". Agreement is only measured when the person typed a value."""
    if tool is None:
        return None
    value = human.get(field)
    if value is None:
        return "blank"
    return "match" if same(tool, value) else "differ"


def pilot_report(vdir: Path, scan: dict[str, Any]) -> Path:
    vdir = Path(vdir)
    manifest, _ = load_manifest(vdir)
    log = read_jsonl(vdir / "review" / "log.jsonl") if (vdir / "review" / "log.jsonl").is_file() else []
    last: dict[str, dict[str, Any]] = {}
    seconds: dict[str, float] = {}
    for e in log:
        last[e["segment_id"]] = e
        seconds[e["segment_id"]] = seconds.get(e["segment_id"], 0.0) + float(e.get("seconds_spent") or 0)

    n = len(last)
    voice_tracks = {e["track_id"] for e in last.values() if e["has_voice"]}
    other_excluded = {e["track_id"] for e in last.values() if e["verdict"] == "exclude" and not e["has_voice"]}
    kept = [e for e in last.values() if not e["has_voice"] and e["verdict"] != "exclude"]
    verdicts = {v: sum(1 for e in kept if e["verdict"] == v) for v in ("correct", "edited", "wrong")}
    halluc = {k: sum(1 for e in last.values() if k in e["hallucinations"]) for k in HALLUCINATION_LABELS}

    agree = {"tempo": {"match": 0, "differ": 0, "blank": 0}, "key": {"match": 0, "differ": 0, "blank": 0}}
    for sid, e in last.items():
        ap = captions.analysis_path(vdir, sid)
        if not ap.is_file():
            continue
        est = read_json(ap).get("estimates") or {}
        meta = e.get("metadata") or {}
        t = _agreement((est.get("tempo") or {}).get("bpm"), meta, "tempo_bpm", lambda a, b: abs(float(a) - float(b)) <= 1)
        k = _agreement((est.get("key") or {}).get("key"), meta, "key", lambda a, b: str(a).lower() == str(b).lower())
        if t:
            agree["tempo"][t] += 1
        if k:
            agree["key"][k] += 1

    per_track: dict[str, int] = {}
    for s in manifest["segments"]:
        per_track[s["track_id"]] = per_track.get(s["track_id"], 0) + 1
    split_tracks = [t for t, c in per_track.items() if c > 1]
    times = list(seconds.values())

    lines = [f"# Báo cáo pilot caption — {manifest['dataset']['name']}/{manifest['dataset']['version']}", "",
             f"Tạo lúc {now_iso()} từ `review/log.jsonl`. Chỉ phản ánh lần duyệt này của người duyệt; "
             "không phải điểm chất lượng nhạc.", "",
             "## Caption máy viết", "",
             f"- Mẫu đã duyệt: {n}",
             f"- Đúng: {verdicts['correct']}/{n}",
             f"- Đã sửa: {verdicts['edited']}/{n}",
             f"- Sai, để nháp: {verdicts['wrong']}/{n}",
             "- Chi tiết bịa: " + ", ".join(f"{HALLUCINATION_LABELS[k]}: {v}" for k, v in halluc.items()),
             f"- Thời gian duyệt: trung vị {statistics.median(times):.1f} s, trung bình {statistics.mean(times):.1f} s mỗi mẫu"
             if times else "- Thời gian duyệt: chưa có", "",
             "## BPM / key do máy ước lượng", "",
             "BPM: trùng {match}, khác {differ}, để trống {blank}".format(**agree["tempo"]),
             "",
             "Key: trùng {match}, khác {differ}, để trống {blank}".format(**agree["key"]), "",
             "(Chỉ tính mẫu máy có đưa ra giá trị. \"Để trống\" là người duyệt không điền, không phải bác bỏ; "
             "chỉ \"trùng\"/\"khác\" cho biết độ đúng của ước lượng.)", "",
             "## Loại bỏ và cảnh báo", "",
             f"- Loại vì giọng: {len(voice_tracks)}",
             f"- Loại lý do khác: {len(other_excluded)}",
             f"- Bài dài hơn một mẫu (> segment.seconds): {len(split_tracks)}",
             f"- Cặp nghi gần trùng khi scan: {len(scan.get('near_duplicates') or [])}",
             f"- File bị loại khi scan: {sum(1 for t in scan.get('tracks') or [] if t.get('exclude_reason'))}"]
    out = vdir / "review" / "pilot-report.md"
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return out
