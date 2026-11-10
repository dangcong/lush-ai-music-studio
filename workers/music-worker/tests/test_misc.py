"""config, audio helpers, gpu parsing."""
from __future__ import annotations

import hashlib
import struct
import wave

import pytest

import lush_worker.audio as audio
from lush_worker.config import ConfigError, load_config, parse_env_file
from lush_worker.engines import build_engines
from lush_worker.gpu import parse_nvidia_smi

REQUIRED = {"CONTROL_PLANE_URL": "https://studio.test/", "NODE_TOKEN": "tok"}


# ---------------- config ----------------
def test_config_defaults():
    c = load_config(REQUIRED, env_file=None)
    assert c.control_plane_url == "https://studio.test" and c.node_token == "tok"
    assert c.engines == ("mock",) and c.acestep_url == "http://127.0.0.1:8001"
    assert c.lora_dir is None and c.mock_step_delay == 0.3 and c.log_level == "INFO"


def test_config_env_file_and_env_override(tmp_path):
    f = tmp_path / ".env"
    f.write_text('# comment\nCONTROL_PLANE_URL="http://file.test"\nNODE_TOKEN=from-file\n'
                 "WORKER_ENGINES = mock, acestep\nexport ACESTEP_TIMEOUT=120\nLORA_DIR='/srv/loras'\n\nbroken line\n",
                 encoding="utf-8")
    assert parse_env_file(f)["CONTROL_PLANE_URL"] == "http://file.test"
    c = load_config({"NODE_TOKEN": "from-env", "MOCK_STEP_DELAY": "0"}, env_file=f)
    assert c.control_plane_url == "http://file.test" and c.node_token == "from-env"
    assert c.engines == ("mock", "acestep") and c.acestep_timeout == 120.0
    assert c.lora_dir.name == "loras" and c.mock_step_delay == 0.0


@pytest.mark.parametrize("env", [
    {"NODE_TOKEN": "t"}, {"CONTROL_PLANE_URL": "http://x"},
    {**REQUIRED, "WORKER_ENGINES": "suno"}, {**REQUIRED, "WORKER_ENGINES": " , "},
    {**REQUIRED, "ACESTEP_TIMEOUT": "soon"}, {**REQUIRED, "MOCK_STEP_DELAY": "-1"},
])
def test_config_errors(env):
    with pytest.raises(ConfigError):
        load_config(env, env_file=None)


def test_build_engines_follows_worker_engines():
    c = load_config({**REQUIRED, "WORKER_ENGINES": "mock,acestep", "MOCK_STEP_DELAY": "0.1"}, env_file=None)
    engines = build_engines(c)
    assert list(engines) == ["mock", "acestep"]
    assert engines["mock"].step_delay == 0.1 and engines["acestep"].name == "acestep"


# ---------------- audio ----------------
def test_sha256_file(tmp_path):
    f = tmp_path / "x.bin"
    f.write_bytes(b"abc" * 500000)
    assert audio.sha256_file(f) == hashlib.sha256(b"abc" * 500000).hexdigest()


def test_probe_wav_pcm(tmp_path):
    f = tmp_path / "a.wav"
    with wave.open(str(f), "wb") as w:
        w.setnchannels(2)
        w.setsampwidth(2)
        w.setframerate(48000)
        w.writeframes(b"\0" * 4 * 24000)
    assert audio.probe_wav(f) == (48000, 0.5)


def test_probe_wav_ieee_float_with_extra_chunk(tmp_path):
    """Format tag 3 (float32) is rejected by the stdlib `wave` module but must be probed fine."""
    rate, channels, frames = 44100, 2, 44100
    data = b"\0" * (frames * channels * 4)
    fmt = struct.pack("<HHIIHH", 3, channels, rate, rate * channels * 4, channels * 4, 32)
    junk = b"LIST" + struct.pack("<I", 3) + b"abc" + b"\0"  # odd-sized chunk + pad byte
    body = b"WAVE" + b"fmt " + struct.pack("<I", len(fmt)) + fmt + junk + b"data" + struct.pack("<I", len(data)) + data
    f = tmp_path / "f.wav"
    f.write_bytes(b"RIFF" + struct.pack("<I", len(body)) + body)
    assert audio.probe_wav(f) == (44100, 1.0)


def test_probe_wav_rejects_garbage(tmp_path):
    f = tmp_path / "x.wav"
    f.write_bytes(b"ID3 definitely not a wav")
    with pytest.raises(ValueError):
        audio.probe_wav(f)


def test_preview_skipped_without_ffmpeg(tmp_path, monkeypatch, caplog):
    monkeypatch.setattr(audio.shutil, "which", lambda name: None)
    dest = tmp_path / "p.mp3"
    with caplog.at_level("WARNING"):
        assert audio.make_mp3_preview(tmp_path / "a.wav", dest) is None
    assert not dest.exists() and "ffmpeg not found" in caplog.text


def test_preview_skipped_when_ffmpeg_fails(tmp_path, monkeypatch):
    import subprocess
    monkeypatch.setattr(audio.shutil, "which", lambda name: "ffmpeg")
    monkeypatch.setattr(audio.subprocess, "run", lambda *a, **k: subprocess.CompletedProcess(a, 1, "", "boom"))
    assert audio.make_mp3_preview(tmp_path / "a.wav", tmp_path / "p.mp3") is None


# ---------------- gpu ----------------
def test_parse_nvidia_smi():
    assert parse_nvidia_smi("NVIDIA GeForce RTX 3060, 37, 512, 12288\n") == {
        "name": "NVIDIA GeForce RTX 3060", "utilization": 37.0, "vram_used_mb": 512, "vram_total_mb": 12288}
    assert parse_nvidia_smi("") is None
    assert parse_nvidia_smi("Some GPU, [N/A], [N/A], [N/A]\n") is None
