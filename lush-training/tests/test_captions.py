import json

import pytest
from conftest import dataset_cfg

from lush_training import captions, dataset
from lush_training.util import LushTrainingError, read_json


@pytest.fixture
def vdir(project, music):
    cfg = dataset_cfg(music)
    dataset.scan(cfg, project)
    dataset.build(cfg, project, "v1")
    return dataset.version_dir(project, "testset", "v1")


def training_segments(vdir):
    manifest, _ = dataset.load_manifest(vdir)
    return [s for s in manifest["segments"] if s["split"] in captions.TRAINING_SPLITS]


def approve_all(vdir):
    for seg in training_segments(vdir):
        captions.set_fields(vdir, seg["segment_id"], by="claude", human=False, metadata={"genre": "lo-fi"},
                            caption_text="lo-fi instrumental, mellow electric piano, soft drums")
        captions.set_fields(vdir, seg["segment_id"], by="Cong", human=True, status="approved")


def test_init_creates_drafts_only_for_train_and_validation(vdir):
    result = captions.init_work(vdir)
    segs = training_segments(vdir)
    assert result == {"created": len(segs), "kept": 0}
    cap = read_json(captions.work_path(vdir, segs[0]["segment_id"]))
    assert cap["status"] == "draft" and cap["caption_text"] is None
    assert all(v is None for v in cap["metadata"].values())          # unknown stays null, nothing is guessed
    analysis = read_json(captions.analysis_path(vdir, segs[0]["segment_id"]))
    assert analysis["segment_sha256"] == segs[0]["sha256"] and analysis["measured"]["duration"] == 5.0
    assert any("không có công cụ nghe hiểu audio" in n for n in analysis["not_available"])
    captions.set_fields(vdir, segs[0]["segment_id"], by="claude", human=False, caption_text="kept text")
    assert captions.init_work(vdir) == {"created": 0, "kept": len(segs)}
    assert read_json(captions.work_path(vdir, segs[0]["segment_id"]))["caption_text"] == "kept text"


def test_analysis_is_cached_by_segment_hash(vdir):
    seg = training_segments(vdir)[0]
    first = captions.analyze_segment(vdir, seg)
    assert captions.analyze_segment(vdir, seg)["analyzed_at"] == first["analyzed_at"]
    assert captions.analyze_segment(vdir, {**seg, "sha256": "0" * 64})["segment_sha256"] == "0" * 64


def test_only_a_human_can_approve(vdir):
    captions.init_work(vdir)
    sid = training_segments(vdir)[0]["segment_id"]
    with pytest.raises(LushTrainingError, match="chỉ người dùng"):
        captions.set_fields(vdir, sid, by="claude", human=False, status="approved")
    captions.set_fields(vdir, sid, by="claude", human=False, caption_text="warm lo-fi instrumental")
    cap = captions.set_fields(vdir, sid, by="Cong", human=True, status="approved", caption_text="warm lo-fi instrumental, tape hiss")
    assert [e["field"] for e in cap["human_edits"]] == ["caption_text", "status"]
    assert cap["human_edits"][0]["old"] == "warm lo-fi instrumental"


def errors_for(vdir, mutate):
    captions.init_work(vdir)
    seg = training_segments(vdir)[0]
    path = captions.work_path(vdir, seg["segment_id"])
    manifest, digest = dataset.load_manifest(vdir)
    cap = captions.empty_caption(manifest, digest, seg)   # start from a clean draft every time
    mutate(cap)
    path.write_text(json.dumps(cap), encoding="utf-8")
    return captions.validate_work(vdir)["errors"].get(seg["segment_id"], [])


def test_validation_catches_schema_hash_and_content_problems(vdir):
    assert any("segment_sha256" in e for e in errors_for(vdir, lambda c: c.update(segment_sha256="f" * 64)))
    assert any("dataset version khác" in e for e in errors_for(vdir, lambda c: c["dataset"].update(manifest_sha256="0" * 64)))
    assert any("thiếu khóa" in e for e in errors_for(vdir, lambda c: c.pop("evidence")))
    assert any("tempo_bpm" in e for e in errors_for(vdir, lambda c: c["metadata"].update(tempo_bpm=900)))
    assert any("vocal" in e for e in errors_for(vdir, lambda c: c.update(caption_text="lo-fi beat with soft female vocals")))
    assert errors_for(vdir, lambda c: c.update(caption_text="lo-fi instrumental, no vocals")) == []
    assert any("BPM" in e for e in errors_for(
        vdir, lambda c: (c["metadata"].update(tempo_bpm=80), c.update(caption_text="jazzy lo-fi at 120 bpm"))))
    assert any("không có bản ghi người duyệt" in e for e in errors_for(
        vdir, lambda c: c.update(status="approved", caption_text="lo-fi instrumental")))
    assert any("evidence" in e for e in errors_for(vdir, lambda c: c["evidence"].append({"field": "genre", "source": "tool"})))


def test_unknown_or_test_split_segment_is_rejected(vdir):
    captions.init_work(vdir)
    manifest, digest = dataset.load_manifest(vdir)
    test_seg = next(s for s in manifest["segments"] if s["split"] == "test")
    cap = captions.empty_caption(manifest, digest, test_seg)
    captions.work_path(vdir, test_seg["segment_id"]).write_text(json.dumps(cap), encoding="utf-8")
    ghost = {**cap, "segment_id": "deadbeefdeadbeef-00000000"}
    captions.work_path(vdir, ghost["segment_id"]).write_text(json.dumps(ghost), encoding="utf-8")
    errors = captions.validate_work(vdir)["errors"]
    assert any("tập test" in e for e in errors[test_seg["segment_id"]])
    assert any("không có trong manifest" in e for e in errors[ghost["segment_id"]])


def test_lock_needs_every_caption_approved_and_is_immutable(vdir):
    captions.init_work(vdir)
    with pytest.raises(LushTrainingError, match="chưa khóa được"):
        captions.lock(vdir, "c1")
    approve_all(vdir)
    info = captions.lock(vdir, "c1")
    segs = training_segments(vdir)
    assert sum(info["segments"].values()) == len(segs)
    engine = read_json(vdir / "captions" / "c1" / "engine.train.json")
    sample = engine["samples"][0]
    assert sample["lyrics"] == "[Instrumental]" and sample["is_instrumental"] is True and sample["labeled"] is True
    assert "bpm" not in sample and "keyscale" not in sample       # unknown -> omitted, never invented
    assert (vdir / "segments" / "train" / sample["filename"]).is_file()
    with pytest.raises(LushTrainingError, match="bất biến"):
        captions.lock(vdir, "c1")
    assert captions.load_lock(vdir, "c1")["captions_sha256"] == info["captions_sha256"]
    locked = vdir / "captions" / "c1" / "captions.json"
    locked.write_text(locked.read_text(encoding="utf-8").replace("mellow", "bright"), encoding="utf-8")
    with pytest.raises(LushTrainingError, match="đã bị sửa"):
        captions.load_lock(vdir, "c1")


def test_known_tempo_and_key_reach_the_engine_sample(vdir):
    captions.init_work(vdir)
    approve_all(vdir)
    seg = training_segments(vdir)[0]
    captions.set_fields(vdir, seg["segment_id"], by="Cong", human=True,
                        metadata={"tempo_bpm": 82, "key": "D minor", "time_signature": "4"})
    cap = read_json(captions.work_path(vdir, seg["segment_id"]))
    sample = captions.engine_sample(cap, seg)
    assert (sample["bpm"], sample["keyscale"], sample["timesignature"], sample["duration"]) == (82, "D minor", "4", 5)
