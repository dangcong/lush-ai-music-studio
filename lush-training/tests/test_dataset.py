import shutil

import numpy as np
import pytest
from conftest import RATE, dataset_cfg, song, write_audio

from lush_training import audio, dataset
from lush_training.util import LushTrainingError, sha256_file


def test_measure_detects_silence_and_clipping():
    quiet = np.zeros((RATE * 4, 2), dtype=np.float32)
    quiet[: RATE] = song(1, 1.0)
    m = audio.measure(quiet, RATE)
    assert m["silence_ratio"] == pytest.approx(0.75, abs=0.03)
    clipped = np.clip(song(2, 2.0) * 20, -1, 1)
    assert audio.measure(clipped, RATE)["clipped_sample_ratio"] > 0.05
    assert audio.measure(song(2, 2.0), RATE)["clipped_sample_ratio"] == 0


def test_scan_reads_formats_recursively_and_never_touches_sources(project, music):
    write_audio(music / "sub" / "deep" / "flac song.flac", song(20))
    write_audio(music / "mp3 song.mp3", song(21))
    (music / "broken.wav").write_bytes(b"RIFF not audio")
    (music / "notes.txt").write_text("ignored")
    write_audio(music / "short.wav", song(22, 2.0))
    before = {p: sha256_file(p) for p in music.rglob("*") if p.is_file()}
    scanned = dataset.scan(dataset_cfg(music), project)
    assert {p: sha256_file(p) for p in music.rglob("*") if p.is_file()} == before
    by_rel = {t["rel"]: t for t in scanned["tracks"]}
    assert "notes.txt" not in by_rel and scanned["files_found"] == 12
    assert by_rel["sub/deep/flac song.flac"]["exclude_reason"] is None
    assert by_rel["mp3 song.mp3"]["metrics"]["sample_rate"] == RATE
    assert "không giải mã được" in by_rel["broken.wav"]["exclude_reason"]
    assert "ngắn hơn" in by_rel["short.wav"]["exclude_reason"]
    assert scanned["usable_tracks"] == 10 and scanned["vocal_screening"] == "none"


def test_duplicates_and_near_duplicates_share_a_group(project, music):
    shutil.copy(music / "song 00.wav", music / "copy of zero.wav")              # byte-identical
    write_audio(music / "quieter.flac", song(1) * 0.4)                           # same audio, other gain + format
    write_audio(music / "song 02 (remix).wav", song(30))                         # related by title only
    scanned = dataset.scan(dataset_cfg(music), project)
    by_rel = {t["rel"]: t for t in scanned["tracks"]}
    assert "trùng từng byte" in by_rel["song 00.wav"]["exclude_reason"] or "trùng từng byte" in by_rel["copy of zero.wav"]["exclude_reason"]
    assert by_rel["quieter.flac"]["group"] == by_rel["song 01.wav"]["group"]
    assert by_rel["song 02 (remix).wav"]["group"] == by_rel["song 02.wav"]["group"]
    pairs = {(n["a"], n["b"]) for n in scanned["near_duplicates"]}
    assert pairs == {("quieter.flac", "song 01.wav")}                            # distinct songs are not flagged
    assert by_rel["song 03.wav"]["group"] != by_rel["song 04.wav"]["group"]


def build(project, music, **over):
    cfg = dataset_cfg(music, **over)
    dataset.scan(cfg, project)
    return cfg, dataset.build(cfg, project, "v1")


def test_build_splits_by_group_without_leakage(project, music):
    write_audio(music / "song 03 v2.wav", song(3) * 0.5)   # near-duplicate of song 03
    write_audio(music / "song 05 (edit).wav", song(40))    # same work as song 05 by title
    cfg, manifest = build(project, music)
    splits_of_group = {}
    for t in manifest["tracks"]:
        splits_of_group.setdefault(t["group"], set()).add(t["split"])
    assert all(len(s) == 1 for s in splits_of_group.values())
    track_split = {t["track_id"]: t["split"] for t in manifest["tracks"]}
    assert all(track_split[s["track_id"]] == s["split"] for s in manifest["segments"])
    assert all(manifest["totals"][s]["segments"] > 0 for s in dataset.SPLITS)
    by_name = {t["source_path"].replace("\\", "/").split("/")[-1]: t for t in manifest["tracks"]}
    assert by_name["song 03 v2.wav"]["split"] == by_name["song 03.wav"]["split"]
    assert by_name["song 05 (edit).wav"]["split"] == by_name["song 05.wav"]["split"]
    # 20 s tracks, 5 s segments
    assert all(t["segments"] == 4 for t in manifest["tracks"])
    vdir = dataset.version_dir(project, "testset", "v1")
    loaded, digest = dataset.load_manifest(vdir, verify_files=True)
    assert loaded == manifest and len(digest) == 64
    assert manifest["vocal_screening"]["tracks_not_screened"] == len(manifest["tracks"])


def test_split_is_deterministic_and_seed_dependent(project, music):
    cfg = dataset_cfg(music)
    tracks = [t for t in dataset.scan(cfg, project)["tracks"]]
    a = dataset.assign_splits(tracks, cfg["split"])
    assert a == dataset.assign_splits(list(reversed(tracks)), cfg["split"])
    assert a != dataset.assign_splits(tracks, {**cfg["split"], "seed": 99})


def test_dataset_version_is_immutable(project, music):
    cfg, _ = build(project, music)
    with pytest.raises(LushTrainingError, match="bất biến"):
        dataset.build(cfg, project, "v1")
    vdir = dataset.version_dir(project, "testset", "v1")
    mpath = vdir / "manifest.json"
    mpath.write_text(mpath.read_text(encoding="utf-8").replace('"instrumental_only": true', '"instrumental_only": false'), encoding="utf-8")
    with pytest.raises(LushTrainingError, match="đã bị sửa"):
        dataset.load_manifest(vdir)


def test_build_requires_rights_and_a_fresh_scan(project, music):
    cfg = dataset_cfg(music, rights={"confirmed": False})
    dataset.scan(cfg, project)
    with pytest.raises(LushTrainingError, match="quyền sử dụng"):
        dataset.build(cfg, project, "v1")
    cfg = dataset_cfg(music)
    write_audio(music / "song 00.wav", song(77))  # file changed after the scan
    with pytest.raises(LushTrainingError, match="đã đổi"):
        dataset.build(cfg, project, "v1")
    assert not dataset.version_dir(project, "testset", "v1").exists()


def test_flagged_vocals_need_a_human_decision(project, music, tmp_path):
    csv = tmp_path / "screen.csv"
    csv.write_text("path,score,tool,tool_version\nsong 01.wav,0.91,detector-x,1.0\nsong 02.wav,0.10,detector-x,1.0\n", encoding="utf-8")
    cfg = dataset_cfg(music, vocal_screening_csv=str(csv))
    scanned = dataset.scan(cfg, project)
    with pytest.raises(LushTrainingError, match="nghi có vocal"):
        dataset.build(cfg, project, "v1")
    sha = {t["rel"]: t["sha256"] for t in scanned["tracks"]}
    with pytest.raises(LushTrainingError):
        dataset.record_review(project, "testset", sha["song 01.wav"], "instrumental", by="")
    dataset.record_review(project, "testset", sha["song 01.wav"], "vocal", by="Cong")
    dataset.record_review(project, "testset", sha["song 03.wav"], "instrumental", by="Cong")
    manifest = dataset.build(cfg, project, "v1")
    states = {t["source_path"].replace("\\", "/").split("/")[-1]: t["vocal"]["state"] for t in manifest["tracks"]}
    assert "song 01.wav" not in states
    assert any("có vocal" in e["reason"] for e in manifest["excluded"])
    assert states["song 02.wav"] == "screened_not_flagged" and states["song 03.wav"] == "confirmed_instrumental"
    assert states["song 04.wav"] == "not_screened"
    assert manifest["vocal_screening"]["method"] == "csv"


def test_too_few_groups_cannot_be_split(project, tmp_path):
    d = tmp_path / "tiny"
    write_audio(d / "a.wav", song(1))
    write_audio(d / "b.wav", song(2))
    cfg = dataset_cfg(d)
    dataset.scan(cfg, project)
    with pytest.raises(LushTrainingError, match="không đủ"):
        dataset.build(cfg, project, "v1")
