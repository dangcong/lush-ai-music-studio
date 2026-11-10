"""GPU stats from `nvidia-smi`. Returns None quietly when there is no NVIDIA GPU / driver."""
from __future__ import annotations

import logging
import shutil
import subprocess
from typing import Any, Optional

log = logging.getLogger(__name__)

_QUERY = "name,utilization.gpu,memory.used,memory.total"


def parse_nvidia_smi(output: str) -> Optional[dict[str, Any]]:
    """Parse the first GPU line of `--format=csv,noheader,nounits` output."""
    for line in output.splitlines():
        parts = [p.strip() for p in line.split(",")]
        if len(parts) != 4:
            continue
        try:
            return {"name": parts[0], "utilization": float(parts[1]),
                    "vram_used_mb": int(float(parts[2])), "vram_total_mb": int(float(parts[3]))}
        except ValueError:  # e.g. "[N/A]"
            continue
    return None


def query_gpu() -> Optional[dict[str, Any]]:
    """First GPU as {name, utilization (percent 0-100), vram_used_mb, vram_total_mb}, or None."""
    exe = shutil.which("nvidia-smi")
    if not exe:
        return None
    try:
        proc = subprocess.run([exe, f"--query-gpu={_QUERY}", "--format=csv,noheader,nounits"],
                              capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.TimeoutExpired) as e:
        log.debug("nvidia-smi failed: %s", e)
        return None
    if proc.returncode != 0:
        log.debug("nvidia-smi exited with %s", proc.returncode)
        return None
    return parse_nvidia_smi(proc.stdout)
