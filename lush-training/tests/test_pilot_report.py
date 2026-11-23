"""Pilot report: caption accuracy, invented details, review time, exclusions, BPM/key agreement."""
from conftest import dataset_cfg

from lush_training import captions, dataset
from lush_training.pilot_report import pilot_report
from lush_training.util import append_jsonl, atomic_write_json


def entry(sid, verdict, *, hallucinations=(), has_voice=False, seconds=10.0, metadata=None, track="t"):
    return {"segment_id": sid, "verdict": verdict, "hallucinations": list(hallucinations), "has_voice": has_voice,
            "seconds_spent": seconds, "metadata": metadata or {}, "track_id": track, "reviewer": "Tester",
            "caption_text": "x", "status": "approved", "at": "2026-11-23T00:00:00Z"}


def test_report_counts(project, music):
    cfg = dataset_cfg(music, split={"validation": 0.2, "test": 0.0, "seed": 1234})
    scanned = dataset.scan(cfg, project)
    dataset.build(cfg, project, "v1")
    vdir = dataset.version_dir(project, "testset", "v1")
    manifest, _ = dataset.load_manifest(vdir)
    sids = [s["segment_id"] for s in manifest["segments"]][:5]
    # analysis said 120 BPM / C major for the first segment; the reviewer kept BPM and cleared the key
    atomic_write_json(captions.analysis_path(vdir, sids[0]),
                      {"estimates": {"tempo": {"bpm": 120.0}, "key": {"key": "C major"}}})
    log = vdir / "review" / "log.jsonl"
    append_jsonl(log, entry(sids[0], "correct", seconds=8, metadata={"tempo_bpm": 120, "key": None}))
    append_jsonl(log, entry(sids[1], "correct", seconds=12))
    append_jsonl(log, entry(sids[2], "correct", seconds=10))
    append_jsonl(log, entry(sids[3], "edited", hallucinations=["instrument"], seconds=40))
    append_jsonl(log, entry(sids[4], "correct", has_voice=True, seconds=20, track="voice-track"))

    text = pilot_report(vdir, scanned).read_text(encoding="utf-8")
    assert "Đúng: 3/5" in text
    assert "Bịa nhạc cụ: 1" in text
    assert "Loại vì giọng: 1" in text
    assert "BPM: giữ 1, sửa 0, xóa 0" in text
    assert "Key: giữ 0, sửa 0, xóa 1" in text
    assert "Thời gian duyệt: trung vị 12.0 s" in text


def test_report_uses_last_decision_per_segment(project, music):
    cfg = dataset_cfg(music, split={"validation": 0.2, "test": 0.0, "seed": 1234})
    scanned = dataset.scan(cfg, project)
    dataset.build(cfg, project, "v1")
    vdir = dataset.version_dir(project, "testset", "v1")
    sid = dataset.load_manifest(vdir)[0]["segments"][0]["segment_id"]
    log = vdir / "review" / "log.jsonl"
    append_jsonl(log, entry(sid, "wrong", seconds=5))
    append_jsonl(log, entry(sid, "edited", seconds=30))
    text = pilot_report(vdir, scanned).read_text(encoding="utf-8")
    assert "Mẫu đã duyệt: 1" in text and "Đã sửa: 1/1" in text
    assert "Thời gian duyệt: trung vị 35.0 s" in text      # time adds up across visits
