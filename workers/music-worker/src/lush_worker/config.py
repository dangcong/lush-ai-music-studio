"""Worker configuration from environment variables and an optional `.env` file."""
from __future__ import annotations

import os
import tempfile
from dataclasses import dataclass
from pathlib import Path
from typing import Mapping, Optional

KNOWN_ENGINES = ("mock", "acestep")


class ConfigError(ValueError):
    pass


def parse_env_file(path: Path) -> dict[str, str]:
    """Minimal `.env` parser: KEY=VALUE lines, `#` comments, optional quotes around the value."""
    values: dict[str, str] = {}
    if not path.is_file():
        return values
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        key = key.strip()
        if key.startswith("export "):
            key = key[len("export "):].strip()
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in "\"'":
            value = value[1:-1]
        values[key] = value
    return values


@dataclass(frozen=True)
class Config:
    control_plane_url: str
    node_token: str
    engines: tuple[str, ...] = ("mock",)
    acestep_url: str = "http://127.0.0.1:8001"
    acestep_api_key: str = ""
    acestep_timeout: float = 1800.0
    lora_dir: Optional[Path] = None
    work_dir: Path = Path(tempfile.gettempdir()) / "lush-worker"
    mock_step_delay: float = 0.3
    log_level: str = "INFO"


def _float(env: Mapping[str, str], key: str, default: float) -> float:
    raw = env.get(key, "").strip()
    if not raw:
        return default
    try:
        value = float(raw)
    except ValueError:
        raise ConfigError(f"{key} must be a number, got {raw!r}") from None
    if value < 0:
        raise ConfigError(f"{key} must be >= 0, got {raw!r}")
    return value


def load_config(env: Optional[Mapping[str, str]] = None, env_file: Optional[Path] = Path(".env")) -> Config:
    """Build a Config. Real environment variables override values from `env_file`."""
    merged: dict[str, str] = {}
    if env_file is not None:
        merged.update(parse_env_file(env_file))
    merged.update(os.environ if env is None else env)

    url = merged.get("CONTROL_PLANE_URL", "").strip().rstrip("/")
    token = merged.get("NODE_TOKEN", "").strip()
    if not url:
        raise ConfigError("CONTROL_PLANE_URL is required (e.g. https://studio.example.com)")
    if not token:
        raise ConfigError("NODE_TOKEN is required (created by an Admin when adding the node)")

    engines = tuple(dict.fromkeys(
        e.strip().lower() for e in merged.get("WORKER_ENGINES", "mock").split(",") if e.strip()))
    if not engines:
        raise ConfigError("WORKER_ENGINES must list at least one engine")
    unknown = [e for e in engines if e not in KNOWN_ENGINES]
    if unknown:
        raise ConfigError(f"WORKER_ENGINES has unknown engine(s) {unknown}; known: {list(KNOWN_ENGINES)}")

    lora_dir = merged.get("LORA_DIR", "").strip()
    work_dir = merged.get("WORK_DIR", "").strip()
    return Config(
        control_plane_url=url,
        node_token=token,
        engines=engines,
        acestep_url=(merged.get("ACESTEP_URL", "").strip() or "http://127.0.0.1:8001").rstrip("/"),
        acestep_api_key=merged.get("ACESTEP_API_KEY", "").strip(),
        acestep_timeout=_float(merged, "ACESTEP_TIMEOUT", 1800.0),
        lora_dir=Path(lora_dir).expanduser() if lora_dir else None,
        work_dir=Path(work_dir).expanduser() if work_dir else Path(tempfile.gettempdir()) / "lush-worker",
        mock_step_delay=_float(merged, "MOCK_STEP_DELAY", 0.3),
        log_level=(merged.get("LOG_LEVEL", "").strip() or "INFO").upper(),
    )
