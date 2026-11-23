"""Captioner output becomes a reviewable DRAFT; it never approves, and voice words force a listen."""
from pathlib import Path

import pytest
from conftest import dataset_cfg

from lush_training import captions, dataset
from lush_training.captioner import CaptionDraft, FolderLabelCaptioner, draft_all
from lush_training.util import read_json


class FakeCaptioner:
    name = "fake"
    revision = "r1"

    def __init__(self, text: str):
        self.text = text
        self.calls: list[Path] = []

    def describe(self, audio: Path) -> str:
        self.calls.append(audio)
        return self.text


@pytest.fixture
def vdir(project, music):
    cfg = dataset_cfg(music, split={"validation": 0.2, "test": 0.0, "seed": 1234})
    dataset.scan(cfg, project)
    dataset.build(cfg, project, "v1")
    v = dataset.version_dir(project, "testset", "v1")
    captions.init_work(v, analyze=False)
    return v


def segment_ids(vdir):
    manifest, _ = dataset.load_manifest(vdir)
    return [s["segment_id"] for s in manifest["segments"] if s["split"] in captions.TRAINING_SPLITS]


def test_draft_sets_text_not_approved(vdir):
    result = draft_all(vdir, FakeCaptioner("mellow lo-fi with warm electric piano and dusty drums"), by="claude")
    ids = segment_ids(vdir)
    assert result == {"drafted": len(ids), "skipped_approved": 0, "needs_listening": 0}
    cap = read_json(captions.work_path(vdir, ids[0]))
    assert cap["status"] == "draft" and cap["author"]["kind"] == "assistant"
    assert cap["caption_text"] == "mellow lo-fi with warm electric piano and dusty drums"
    ev = [e for e in cap["evidence"] if e["field"] == "caption_text"]
    assert ev and ev[0]["source"] == "tool" and ev[0]["tool"] == "fake@r1"


def test_draft_with_voice_words_needs_listening(vdir):
    result = draft_all(vdir, FakeCaptioner("warm keys with a soft vocal-like pad"), by="claude")
    assert result["needs_listening"] == len(segment_ids(vdir))
    cap = read_json(captions.work_path(vdir, segment_ids(vdir)[0]))
    assert cap["status"] == "needs_listening"


def test_negated_voice_words_do_not_flag(vdir):
    draft = CaptionDraft.from_text("instrumental lo-fi beat, no vocals", source="fake@r1")
    assert draft.voice_terms == []


def test_draft_skips_approved(vdir):
    sid = segment_ids(vdir)[0]
    captions.set_fields(vdir, sid, by="Tester", human=True, caption_text="human text", status="approved")
    fake = FakeCaptioner("machine text")
    result = draft_all(vdir, fake, by="claude")
    assert result["skipped_approved"] == 1
    assert read_json(captions.work_path(vdir, sid))["caption_text"] == "human text"
    assert len(fake.calls) == len(segment_ids(vdir)) - 1


def test_draft_over_150_words_needs_listening(vdir):
    long_text = " ".join(["mellow"] * 151)
    result = draft_all(vdir, FakeCaptioner(long_text), by="claude")
    assert result["needs_listening"] == len(segment_ids(vdir))
    cap = read_json(captions.work_path(vdir, segment_ids(vdir)[0]))
    assert cap["status"] == "needs_listening" and cap["caption_text"] == long_text   # not silently cut


def test_folder_label_captioner():
    c = FolderLabelCaptioner("lo-fi instrumental, chill beat")
    assert c.describe(Path("x.wav")) == "lo-fi instrumental, chill beat"
    assert c.name == "folder-label"
