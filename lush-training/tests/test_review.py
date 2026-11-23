"""Review page logic: human decisions, whole-track exclusion, and caption carry-over to a rebuilt version."""
import json
import threading
import urllib.error
import urllib.request

import pytest
from conftest import dataset_cfg

from lush_training import captions, dataset
from lush_training.review import ReviewDecision, apply_decision, make_server
from lush_training.util import read_json, read_jsonl


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


def train_val(vdir):
    manifest, _ = dataset.load_manifest(vdir)
    return [s for s in manifest["segments"] if s["split"] in captions.TRAINING_SPLITS]


def decision(sid, **kw):
    base = {"segment_id": sid, "verdict": "correct", "caption_text": "lo-fi instrumental, warm keys, soft drums",
            "metadata": {"genre": "lo-fi"}, "hallucinations": [], "has_voice": False, "seconds_spent": 12.5,
            "reviewer": "Tester"}
    base.update(kw)
    return ReviewDecision(**base)


def test_apply_correct_approves(project, vdir):
    sid = train_val(vdir)[0]["segment_id"]
    apply_decision(project, "testset", vdir, decision(sid))
    cap = read_json(captions.work_path(vdir, sid))
    assert cap["status"] == "approved" and cap["author"]["kind"] == "human"
    assert cap["metadata"]["genre"] == "lo-fi"


def test_apply_wrong_keeps_draft(project, vdir):
    sid = train_val(vdir)[0]["segment_id"]
    apply_decision(project, "testset", vdir, decision(sid, verdict="wrong", hallucinations=["instrument"]))
    assert read_json(captions.work_path(vdir, sid))["status"] == "draft"


def test_apply_voice_excludes_track_on_next_build(project, cfg, vdir):
    seg = train_val(vdir)[0]
    apply_decision(project, "testset", vdir, decision(seg["segment_id"], has_voice=True))
    assert read_json(captions.work_path(vdir, seg["segment_id"]))["status"] == "needs_listening"
    manifest = dataset.build(cfg, project, "v2")
    track_sha = next(t["sha256"] for t in dataset.load_manifest(vdir)[0]["tracks"] if t["track_id"] == seg["track_id"])
    assert any(e["sha256"] == track_sha and "vocal" in e["reason"] for e in manifest["excluded"])


def test_apply_exclude_without_voice_excludes_track(project, cfg, vdir):
    seg = train_val(vdir)[0]
    apply_decision(project, "testset", vdir, decision(seg["segment_id"], verdict="exclude"))
    manifest = dataset.build(cfg, project, "v2")
    assert any("người duyệt loại" in e["reason"] for e in manifest["excluded"])


def test_review_post_is_atomic_and_logged(project, vdir):
    sid = train_val(vdir)[0]["segment_id"]
    apply_decision(project, "testset", vdir, decision(sid, verdict="edited", caption_text="first text"))
    apply_decision(project, "testset", vdir, decision(sid, verdict="edited", caption_text="second text"))
    cap = read_json(captions.work_path(vdir, sid))
    assert cap["caption_text"] == "second text"
    texts = [e["new"] for e in cap["human_edits"] if e["field"] == "caption_text"]
    assert texts == ["first text", "second text"]
    log = read_jsonl(vdir / "review" / "log.jsonl")
    assert len(log) == 2 and log[-1]["caption_text"] == "second text" and "at" in log[-1]


def test_decision_rejects_unknown_metadata_field():
    with pytest.raises(ValueError):
        decision("x", metadata={"singer": "someone"})


def test_carry_over_keeps_approved(project, cfg, vdir):
    segs = train_val(vdir)
    for s in segs:
        apply_decision(project, "testset", vdir, decision(s["segment_id"]))
    apply_decision(project, "testset", vdir, decision(segs[0]["segment_id"], has_voice=True))
    dataset.build(cfg, project, "v2")
    v2 = dataset.version_dir(project, "testset", "v2")
    result = captions.carry_over(vdir, v2)
    assert result["carried"] >= 1
    report = captions.validate_work(v2)
    assert report["ready_to_lock"], report


@pytest.fixture
def server(project, vdir):
    srv = make_server(project, "testset", vdir, host="127.0.0.1", port=0)
    thread = threading.Thread(target=srv.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{srv.server_address[1]}"
    srv.shutdown()
    srv.server_close()


def test_audio_route_rejects_unknown_id(server):
    for path in ("/audio/..%2Fmanifest", "/audio/../manifest.json", "/audio/nope"):
        with pytest.raises(urllib.error.HTTPError) as e:
            urllib.request.urlopen(server + path)
        assert e.value.code == 404


def test_items_and_post_roundtrip(server, vdir):
    items = json.load(urllib.request.urlopen(server + "/api/items"))
    sid = items[0]["segment_id"]
    audio = urllib.request.urlopen(server + f"/audio/{sid}")
    assert audio.headers["Content-Type"] == "audio/wav" and len(audio.read()) > 1000
    body = json.dumps(decision(sid).model_dump()).encode()
    req = urllib.request.Request(server + f"/api/items/{sid}", data=body, method="POST",
                                 headers={"Content-Type": "application/json"})
    assert json.load(urllib.request.urlopen(req))["status"] == "approved"
    bad = urllib.request.Request(server + f"/api/items/{sid}", data=b'{"verdict": "maybe"}', method="POST",
                                 headers={"Content-Type": "application/json"})
    with pytest.raises(urllib.error.HTTPError) as e:
        urllib.request.urlopen(bad)
    assert e.value.code == 422
