"""Read-only environment check. Installs nothing, changes nothing, never touches the GPU beyond
asking `nvidia-smi` and (if torch is importable) `torch.cuda` for facts."""
from __future__ import annotations

import importlib
import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path
from typing import Any, Optional

from .config import ENV_DATA_ROOT, Project

OK, WARN, FAIL, INFO = "ok", "warn", "fail", "info"


def _check(name: str, status: str, detail: str, **data: Any) -> dict[str, Any]:
    return {"name": name, "status": status, "detail": detail, **data}


def ram_bytes() -> Optional[tuple[int, int]]:
    """(total, available) physical memory, or None when the platform does not tell."""
    if sys.platform == "win32":
        import ctypes

        class MemoryStatus(ctypes.Structure):
            _fields_ = [("dwLength", ctypes.c_ulong), ("dwMemoryLoad", ctypes.c_ulong),
                        ("ullTotalPhys", ctypes.c_ulonglong), ("ullAvailPhys", ctypes.c_ulonglong),
                        ("ullTotalPageFile", ctypes.c_ulonglong), ("ullAvailPageFile", ctypes.c_ulonglong),
                        ("ullTotalVirtual", ctypes.c_ulonglong), ("ullAvailVirtual", ctypes.c_ulonglong),
                        ("ullAvailExtendedVirtual", ctypes.c_ulonglong)]
        status = MemoryStatus()
        status.dwLength = ctypes.sizeof(MemoryStatus)
        if ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
            return int(status.ullTotalPhys), int(status.ullAvailPhys)
        return None
    try:
        info = {}
        for line in Path("/proc/meminfo").read_text().splitlines():
            key, _, rest = line.partition(":")
            info[key] = int(rest.split()[0]) * 1024
        return info["MemTotal"], info.get("MemAvailable", info["MemFree"])
    except (OSError, KeyError, ValueError, IndexError):
        return None


def query_gpus() -> Optional[list[dict[str, Any]]]:
    """GPUs as nvidia-smi reports them (whole machine, not one process). None = nvidia-smi unavailable."""
    exe = shutil.which("nvidia-smi")
    if not exe:
        return None
    fields = "index,name,memory.total,memory.used,utilization.gpu,temperature.gpu,driver_version"
    try:
        proc = subprocess.run([exe, f"--query-gpu={fields}", "--format=csv,noheader,nounits"],
                              capture_output=True, text=True, timeout=15)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if proc.returncode != 0:
        return None
    gpus = []
    for line in proc.stdout.strip().splitlines():
        parts = [p.strip() for p in line.split(",")]
        if len(parts) != 7:
            continue

        def num(v: str) -> Optional[int]:
            try:
                return int(float(v))
            except ValueError:
                return None
        gpus.append({"index": num(parts[0]), "name": parts[1], "memory_total_mib": num(parts[2]),
                     "memory_used_mib": num(parts[3]), "utilization_percent": num(parts[4]),
                     "temperature_c": num(parts[5]), "driver": parts[6]})
    return gpus


def torch_facts() -> dict[str, Any]:
    try:
        torch = importlib.import_module("torch")
    except Exception as e:  # torch missing or broken: both are facts worth reporting
        return {"installed": False, "error": f"{type(e).__name__}: {e}"}
    facts: dict[str, Any] = {"installed": True, "version": torch.__version__, "cuda_build": torch.version.cuda,
                             "cuda_available": bool(torch.cuda.is_available())}
    if facts["cuda_available"]:
        facts["devices"] = [torch.cuda.get_device_name(i) for i in range(torch.cuda.device_count())]
        try:
            facts["bf16_supported"] = bool(torch.cuda.is_bf16_supported())
        except Exception:
            facts["bf16_supported"] = None
    return facts


def module_version(name: str) -> Optional[str]:
    try:
        return str(getattr(importlib.import_module(name), "__version__", "installed"))
    except Exception:
        return None


def run_doctor(project: Optional[Project], project_error: Optional[str] = None) -> dict[str, Any]:
    checks: list[dict[str, Any]] = []
    py = sys.version_info
    checks.append(_check("python", OK if (3, 11) <= py[:2] < (3, 13) else WARN,
                         f"{platform.python_version()} ({sys.executable}). ACE-Step ca1e85f cần >=3.11,<3.13; "
                         "phần dataset/caption/export chạy được trên bản mới hơn.",
                         version=platform.python_version()))
    checks.append(_check("os", INFO, f"{platform.system()} {platform.release()} ({platform.machine()})"))

    ram = ram_bytes()
    checks.append(_check("ram", INFO if ram else WARN,
                         f"tổng {ram[0] / 2**30:.1f} GiB, trống {ram[1] / 2**30:.1f} GiB" if ram else "không đọc được",
                         total_bytes=ram[0] if ram else None, available_bytes=ram[1] if ram else None))

    gpus = query_gpus()
    if gpus is None:
        checks.append(_check("gpu", WARN, "không có nvidia-smi: không xác minh được GPU trên máy này", gpus=[]))
    elif not gpus:
        checks.append(_check("gpu", FAIL, "nvidia-smi chạy nhưng không báo GPU nào", gpus=[]))
    else:
        text = "; ".join(f"#{g['index']} {g['name']}: {g['memory_used_mib']}/{g['memory_total_mib']} MiB đang dùng, "
                         f"{g['utilization_percent']}%, driver {g['driver']}" for g in gpus)
        checks.append(_check("gpu", OK, text + ". Nhiều GPU không cộng gộp VRAM cho một model.", gpus=gpus))

    torch = torch_facts()
    if not torch["installed"]:
        checks.append(_check("torch", WARN, "chưa cài trong môi trường Python này. Trainer phải chạy bằng Python của "
                             "virtualenv ACE-Step (acestep.python trong project.yaml).", **torch))
    elif not torch["cuda_available"]:
        checks.append(_check("torch", FAIL, f"torch {torch['version']} không thấy CUDA", **torch))
    else:
        checks.append(_check("torch", OK, f"torch {torch['version']} (CUDA {torch['cuda_build']}), "
                             f"bf16: {torch.get('bf16_supported')}", **torch))

    sf = module_version("soundfile")
    if sf:
        import soundfile
        formats = soundfile.available_formats()
        missing = [f for f in ("WAV", "FLAC", "MP3") if f not in formats]
        checks.append(_check("audio_decode", FAIL if missing else OK,
                             f"soundfile {sf}, libsndfile {soundfile.__libsndfile_version__}"
                             + (f"; KHÔNG giải mã được: {missing}" if missing else "; WAV/FLAC/MP3 đều đọc được")))
    else:
        checks.append(_check("audio_decode", FAIL, "chưa cài soundfile (pip install -e .)"))
    optional = {name: module_version(name) for name in ("librosa", "pyloudnorm", "safetensors", "bitsandbytes")}
    checks.append(_check("optional_packages", INFO, ", ".join(f"{k}: {v or 'chưa cài'}" for k, v in optional.items()),
                         packages=optional))

    if project is None:
        checks.append(_check("data_root", FAIL, project_error or f"chưa cấu hình {ENV_DATA_ROOT}"))
    else:
        root = project.data_root
        if not root.is_dir():
            checks.append(_check("data_root", FAIL, f"{root} chưa tồn tại (tạo thư mục trước; doctor không tự tạo)"))
        else:
            usage = shutil.disk_usage(root)
            writable = os.access(root, os.W_OK)
            checks.append(_check("data_root", OK if writable else FAIL,
                                 f"{root}: trống {usage.free / 2**30:.1f} GiB / {usage.total / 2**30:.1f} GiB"
                                 + ("" if writable else "; KHÔNG ghi được"), free_bytes=usage.free))
        from .acestep_probe import probe_repo
        if project.acestep_repo is None:
            checks.append(_check("acestep", WARN, "chưa khai báo acestep.repo_dir trong project.yaml"))
        else:
            probe = probe_repo(project.acestep_repo, project.acestep_commit)
            checks.append(_check("acestep", OK if probe["ok"] else FAIL,
                                 "; ".join(probe["problems"]) or f"commit {probe['commit']} khớp, đủ symbol trainer cần",
                                 probe=probe))
            if project.acestep_python and not project.acestep_python.is_file():
                checks.append(_check("acestep_python", FAIL, f"không có {project.acestep_python}"))
        ckpt = project.checkpoints_dir
        found = sorted(p.name for p in ckpt.iterdir() if p.is_dir()) if ckpt.is_dir() else []
        checks.append(_check("pretrained", OK if found else WARN,
                             f"{ckpt}: {', '.join(found) if found else 'chưa có checkpoint nào (tải cần phê duyệt)'}",
                             checkpoints=found))
    worst = FAIL if any(c["status"] == FAIL for c in checks) else WARN if any(c["status"] == WARN for c in checks) else OK
    return {"status": worst, "checks": checks}


def format_report(report: dict[str, Any]) -> str:
    marks = {OK: "[OK]  ", WARN: "[WARN]", FAIL: "[FAIL]", INFO: "[INFO]"}
    lines = [f"{marks[c['status']]} {c['name']}: {c['detail']}" for c in report["checks"]]
    lines.append(f"Kết luận: {report['status'].upper()}. Đây là kiểm tra môi trường, không chứng minh model train "
                 "vừa GPU: việc đó phải đo bằng một lần chạy thử được phê duyệt.")
    return "\n".join(lines)
