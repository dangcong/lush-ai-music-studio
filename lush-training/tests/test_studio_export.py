"""Export a locked dataset version to the layout Lush Studio's Train Style worker reads:
`<studio_root>/<name>/<version>/{dataset.json, audio/, manifest.json}`."""
import pytest
from conftest import dataset_cfg, song, write_audio

from lush_training import captions, dataset
from lush_training.studio_export import export_studio
from lush_training.util import LushTrainingError, read_json


def approve_all(vdir):
    manifest, _ = dataset.load_manifest(vdir)
    captions.init_work(vdir, analyze=False)
    for seg in manifest["segments"]:
        if seg["split"] not in captions.TRAINING_SPLITS:
            continue
        captions.set_fields(vdir, seg["segment_id"], by="claude", human=False, metadata={"genre": "lo-fi"},
                            caption_text="lo-fi instrumental, mellow electric piano, soft drums")
        captions.set_fields(vdir, seg["segment_id"], by="Tester", human=True, status="approved")


def locked_version(project, cfg, version="v1"):
    dataset.scan(cfg, project)
    dataset.build(cfg, project, version)
    vdir = dataset.version_dir(project, cfg["name"], version)
    approve_all(vdir)
    captions.lock(vdir, "c1")
    return vdir


def test_export_layout(project, music, tmp_path):
    cfg = dataset_cfg(music, segment={"seconds": 240.0, "min_seconds": 3.0, "max_silence_ratio": 0.5},
                      split={"validation": 0.2, "test": 0.0, "seed": 1234})
    vdir = locked_version(project, cfg)
    result = export_studio(vdir, "c1", tmp_path / "studio", "lofi-pilot/v1")

    out = tmp_path / "studio" / "lofi-pilot" / "v1"
    assert result["dir"] == str(out)
    manifest, _ = dataset.load_manifest(vdir)
    train = [s for s in manifest["segments"] if s["split"] == "train"]
    validation = [s for s in manifest["segments"] if s["split"] == "validation"]
    assert train and validation

    ds = read_json(out / "dataset.json")
    assert len(ds["samples"]) == len(train) == result["train_samples"]
    assert ds["metadata"]["name"] == "lofi-pilot-v1"
    for sample in ds["samples"]:
        assert sample["lyrics"] == "[Instrumental]"
        assert (out / "audio" / sample["audio_path"]).is_file()

    exported = read_json(out / "manifest.json")
    assert exported["caption_version"] == "c1"
    assert sorted(exported["validation_segments"]) == sorted(s["segment_id"] for s in validation)
    assert result["excluded_validation"] == exported["validation_segments"]
    assert {t["sha256"] for t in exported["sources"]} == {t["sha256"] for t in manifest["tracks"] if t["split"] == "train"}


def test_export_refuses_existing(project, music, tmp_path):
    cfg = dataset_cfg(music, split={"validation": 0.2, "test": 0.0, "seed": 1234})
    vdir = locked_version(project, cfg)
    export_studio(vdir, "c1", tmp_path / "studio", "lofi-pilot/v1")
    with pytest.raises(LushTrainingError, match="đã tồn tại"):
        export_studio(vdir, "c1", tmp_path / "studio", "lofi-pilot/v1")


def test_export_rejects_bad_key(project, music, tmp_path):
    cfg = dataset_cfg(music, split={"validation": 0.2, "test": 0.0, "seed": 1234})
    vdir = locked_version(project, cfg)
    for key in ("../x", "a/b/c", "", "lofi pilot"):
        with pytest.raises(LushTrainingError, match="key"):
            export_studio(vdir, "c1", tmp_path / "studio", key)
    assert not (tmp_path / "studio").exists() or not any((tmp_path / "studio").iterdir())


def test_export_reports_split_tracks(project, tmp_path):
    music = tmp_path / "long"
    write_audio(music / "long.wav", song(1, seconds=300.0))
    cfg = dataset_cfg(music, segment={"seconds": 240.0, "min_seconds": 30.0, "max_silence_ratio": 0.5},
                      split={"validation": 0.0, "test": 0.0, "seed": 1234})
    vdir = locked_version(project, cfg)
    result = export_studio(vdir, "c1", tmp_path / "studio", "lofi-long/v1")
    manifest, _ = dataset.load_manifest(vdir)
    assert result["train_samples"] == 2
    assert result["split_tracks"] == [manifest["tracks"][0]["track_id"]]


def test_scan_unicode_mp3_name(project, tmp_path):
    music = tmp_path / "mp3"
    write_audio(music / "Lofi chill – đêm 01.mp3", song(3, seconds=20.0), format="MP3")
    write_audio(music / "Lofi chill – đêm 02.mp3", song(4, seconds=20.0), format="MP3")
    cfg = dataset_cfg(music, split={"validation": 0.0, "test": 0.0, "seed": 1234})
    vdir = locked_version(project, cfg)
    export_studio(vdir, "c1", tmp_path / "studio", "lofi-mp3/v1")
    ds = read_json(tmp_path / "studio" / "lofi-mp3" / "v1" / "dataset.json")
    manifest, _ = dataset.load_manifest(vdir)
    ids = {s["segment_id"] for s in manifest["segments"]}
    for sample in ds["samples"]:
        assert sample["audio_path"].endswith(".wav") and sample["audio_path"][:-4] in ids
        assert "đêm" not in sample["audio_path"]
