"""Audio helpers: file checksum, WAV header probing, MP3 preview via ffmpeg."""
from __future__ import annotations

import hashlib
import logging
import shutil
import struct
import subprocess
from pathlib import Path
from typing import Optional

log = logging.getLogger(__name__)

PREVIEW_BITRATE = "192k"


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def probe_wav(path: Path) -> tuple[int, float]:
    """Return (sample_rate, duration_seconds) from a RIFF/WAVE header.

    Parses the chunks directly instead of using the `wave` module so that IEEE-float WAV files
    (format tag 3, which `wave` rejects) are handled too. Raises ValueError if not a WAV file.
    """
    with open(path, "rb") as f:
        header = f.read(12)
        if len(header) < 12 or header[:4] != b"RIFF" or header[8:12] != b"WAVE":
            raise ValueError(f"{path.name} is not a RIFF/WAVE file")
        sample_rate = byte_rate = 0
        while True:
            chunk = f.read(8)
            if len(chunk) < 8:
                break
            chunk_id, size = chunk[:4], struct.unpack("<I", chunk[4:])[0]
            if chunk_id == b"fmt ":
                fmt = f.read(size)
                if len(fmt) < 16:
                    raise ValueError(f"{path.name}: truncated fmt chunk")
                _tag, _channels, sample_rate, byte_rate = struct.unpack("<HHII", fmt[:12])
                f.seek(size & 1, 1)
            elif chunk_id == b"data":
                if not sample_rate or not byte_rate:
                    raise ValueError(f"{path.name}: data chunk before a valid fmt chunk")
                # Streamed writers may leave a placeholder size; trust the real file size then.
                remaining = path.stat().st_size - f.tell()
                data_bytes = size if 0 < size <= remaining else remaining
                return sample_rate, data_bytes / byte_rate
            else:
                f.seek(size + (size & 1), 1)
    raise ValueError(f"{path.name}: no data chunk found")


def ffmpeg_path() -> Optional[str]:
    return shutil.which("ffmpeg")


def make_mp3_preview(wav: Path, dest: Path, timeout: float = 300.0) -> Optional[Path]:
    """Encode `wav` to MP3 with ffmpeg. Returns None (and logs a warning) when ffmpeg is missing
    or fails; a fake MP3 is never produced."""
    ffmpeg = ffmpeg_path()
    if not ffmpeg:
        log.warning("ffmpeg not found in PATH: skipping the audio_preview (MP3) artifact")
        return None
    cmd = [ffmpeg, "-y", "-loglevel", "error", "-i", str(wav), "-vn",
           "-codec:a", "libmp3lame", "-b:a", PREVIEW_BITRATE, str(dest)]
    try:
        proc = subprocess.run(cmd, capture_output=True, text=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as e:
        log.warning("ffmpeg failed to run (%s): skipping audio_preview", e)
        return None
    if proc.returncode != 0 or not dest.is_file() or dest.stat().st_size == 0:
        log.warning("ffmpeg exited with %s: %s; skipping audio_preview",
                    proc.returncode, (proc.stderr or "").strip()[-500:])
        dest.unlink(missing_ok=True)
        return None
    return dest
