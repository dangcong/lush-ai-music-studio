"""Regression tests for the final-review findings (fix pass)."""
import json
import threading
import urllib.request

import pytest
from conftest import dataset_cfg, song, write_audio

from lush_training import captions, dataset, studio_export
from lush_training.captioner import CaptionDraft, draft_all
from lush_training.review import ReviewDecision, apply_decision, list_items, make_server
from lush_training.studio_export import export_studio
from lush_training.util import LushTrainingError, read_json, read_jsonl


class Fake:
    name, revision = "fake", "r1"

    def __init__(self, text):
        self.text = text

    def describe(self, audio):
        return self.text


@pytest.fixture
def cfg(music):
    return dataset_cfg(music, split={"validation": 0.2, "test": 0.0, "seed": 1234})


@pytest.fixture
def vdir(project, cfg):
    dataset.scan(cfg, project)
    dataset.build(cfg, project, "v1")
    v = dataset.version_dir(project, "testset", "v1")
    captions.init_work(v, analyze=False)
    return v


def segs(vdir):
    manifest, _ = dataset.load_manifest(vdir)
    return [s for s in manifest["segments"] if s["split"] in captions.TRAINING_SPLITS]


def ok(sid, **kw):
    base = {"segment_id": sid, "verdict": "correct", "caption_text": "lo-fi instrumental, warm keys, soft drums",
            "metadata": {}, "hallucinations": [], "has_voice": False, "seconds_spent": 5, "reviewer": "Tester"}
    base.update(kw)
    return ReviewDecision(**base)


def approve_all(project, vdir):
    for s in segs(vdir):
        apply_decision(project, "testset", vdir, ok(s["segment_id"]))


# ---- Important 1: an excluded track cannot be approved, locked or exported in the current version ----

def test_apply_refuses_approve_on_excluded_track(project, vdir):
    sid = segs(vdir)[0]["segment_id"]
    apply_decision(project, "testset", vdir, ok(sid, has_voice=True))
    with pytest.raises(LushTrainingError, match="đã bị loại"):
        apply_decision(project, "testset", vdir, ok(sid))
    assert read_json(captions.work_path(vdir, sid))["status"] == "needs_listening"


def test_lock_refuses_excluded_track(project, vdir):
    approve_all(project, vdir)
    track = next(t for t in dataset.load_manifest(vdir)[0]["tracks"] if t["track_id"] == segs(vdir)[0]["track_id"])
    dataset.record_review(project, "testset", track["sha256"], "vocal", by="Tester")
    with pytest.raises(LushTrainingError, match="bị loại"):
        captions.lock(vdir, "c1")


def test_export_refuses_track_excluded_after_lock(project, vdir, tmp_path):
    approve_all(project, vdir)
    captions.lock(vdir, "c1")
    track = next(t for t in dataset.load_manifest(vdir)[0]["tracks"] if t["split"] == "train")
    dataset.record_review(project, "testset", track["sha256"], "excluded", by="Tester")
    with pytest.raises(LushTrainingError, match="bị loại"):
        export_studio(vdir, "c1", tmp_path / "studio", "lofi/v1")


def test_draft_does_not_override_human_decision(project, vdir):
    sid = segs(vdir)[0]["segment_id"]
    apply_decision(project, "testset", vdir, ok(sid, verdict="wrong", caption_text="human wording"))
    draft_all(vdir, Fake("machine wording"), by="claude")
    cap = read_json(captions.work_path(vdir, sid))
    assert cap["caption_text"] == "human wording"


# ---- Important 2: concurrent saves ----

def test_concurrent_posts_keep_all_edits(project, vdir):
    sid = segs(vdir)[0]["segment_id"]
    texts = [f"lo-fi instrumental take {i}" for i in range(8)]
    threads = [threading.Thread(target=apply_decision, args=(project, "testset", vdir, ok(sid, verdict="edited", caption_text=t)))
               for t in texts]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    cap = read_json(captions.work_path(vdir, sid))
    edits = [e["new"] for e in cap["human_edits"] if e["field"] == "caption_text"]
    assert sorted(edits) == sorted(texts) and cap["caption_text"] == edits[-1]
    assert len(read_jsonl(vdir / "review" / "log.jsonl")) == 8


def test_concurrent_exclusions_keep_both(project, vdir):
    by_track = {}
    for s in segs(vdir):
        by_track.setdefault(s["track_id"], s["segment_id"])
    a, b = list(by_track.values())[:2]
    threads = [threading.Thread(target=apply_decision, args=(project, "testset", vdir, ok(sid, verdict="exclude")))
               for sid in (a, b)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    assert len(dataset.load_reviews(project, "testset")) == 2


# ---- Important 3: rebuild keeps the previous splits ----

def test_rebuild_pins_previous_splits(project, music):
    cfg = dataset_cfg(music, split={"validation": 0.2, "test": 0.2, "seed": 1234})
    dataset.scan(cfg, project)
    v1 = dataset.build(cfg, project, "v1")
    before = {t["track_id"]: t["split"] for t in v1["tracks"]}
    for victim in v1["tracks"]:
        dataset.record_review(project, "testset", victim["sha256"], "excluded", by="Tester")
        v2 = dataset.build(cfg, project, f"v2-{victim['track_id'][:6]}", previous="v1")
        after = {t["track_id"]: t["split"] for t in v2["tracks"]}
        assert victim["track_id"] not in after
        assert all(after[t] == before[t] for t in after), (victim["track_id"], before, after)
        dataset.record_review(project, "testset", victim["sha256"], "instrumental", by="Tester")


# ---- Important 5: reasons shown, invalid approval refused ----

def test_items_show_draft_reasons(project, vdir):
    draft_all(vdir, Fake("warm keys with a vocal-like choir pad"), by="claude")
    item = list_items(vdir)[0]
    assert item["status"] == "needs_listening" and any("giọng" in r for r in item["reasons"])


def test_approve_invalid_caption_refused(project, vdir):
    sid = segs(vdir)[0]["segment_id"]
    with pytest.raises(LushTrainingError, match="vocal"):
        apply_decision(project, "testset", vdir, ok(sid, caption_text="lo-fi beat with soft vocals"))
    assert read_json(captions.work_path(vdir, sid))["status"] == "draft"


# ---- re-graded minors ----

def test_vox_and_speech_terms_flag():
    for text in ("lofi beat with vox chops", "a soft chant over pads", "spoken speech sample", "whispered texture"):
        assert CaptionDraft.from_text(text, source="x").voice_terms, text


def test_export_requires_name_and_version(project, vdir, tmp_path):
    approve_all(project, vdir)
    captions.lock(vdir, "c1")
    with pytest.raises(LushTrainingError, match="key"):
        export_studio(vdir, "c1", tmp_path / "studio", "lofi")


def test_export_staging_invisible_to_worker(project, vdir, tmp_path, monkeypatch):
    approve_all(project, vdir)
    captions.lock(vdir, "c1")
    root = tmp_path / "studio"
    real = studio_export.atomic_write_json
    seen = []

    def spy(path, obj):
        real(path, obj)
        if path.name == "manifest.json":
            seen.extend(root.glob("*/*/dataset.json"))
    monkeypatch.setattr(studio_export, "atomic_write_json", spy)
    export_studio(vdir, "c1", root, "lofi/v1")
    assert seen == []                       # nothing worker-visible before the final rename
    assert list(root.glob("*/*/dataset.json")) == [root / "lofi" / "v1" / "dataset.json"]


def test_export_reports_truncated_tracks(project, tmp_path):
    music = tmp_path / "long"
    write_audio(music / "long.wav", song(1, seconds=260.0))
    cfg = dataset_cfg(music, segment={"seconds": 240.0, "min_seconds": 30.0, "max_silence_ratio": 0.5},
                      split={"validation": 0.0, "test": 0.0, "seed": 1})
    dataset.scan(cfg, project)
    dataset.build(cfg, project, "v1")
    v = dataset.version_dir(project, "testset", "v1")
    captions.init_work(v, analyze=False)
    for s in segs(v):
        apply_decision(project, "testset", v, ok(s["segment_id"]))
    captions.lock(v, "c1")
    result = export_studio(v, "c1", tmp_path / "studio", "lofi-long/v1")
    assert len(result["truncated_tracks"]) == 1
    assert abs(result["truncated_tracks"][0]["dropped_seconds"] - 20.0) < 0.5


@pytest.fixture
def server(project, vdir):
    srv = make_server(project, "testset", vdir, host="127.0.0.1", port=0)
    threading.Thread(target=srv.serve_forever, daemon=True).start()
    yield f"http://127.0.0.1:{srv.server_address[1]}"
    srv.shutdown()
    srv.server_close()


def test_audio_supports_range(server):
    sid = json.load(urllib.request.urlopen(server + "/api/items"))[0]["segment_id"]
    req = urllib.request.Request(server + f"/audio/{sid}", headers={"Range": "bytes=0-99"})
    r = urllib.request.urlopen(req)
    assert r.status == 206 and len(r.read()) == 100
    assert r.headers["Accept-Ranges"] == "bytes" and r.headers["Content-Range"].startswith("bytes 0-99/")
