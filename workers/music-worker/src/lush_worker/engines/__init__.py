"""Engine registry: build the engines enabled by WORKER_ENGINES."""
from __future__ import annotations

from ..config import Config
from .base import Engine


def build_engines(config: Config) -> dict[str, Engine]:
    engines: dict[str, Engine] = {}
    for name in config.engines:
        if name == "mock":
            from .mock import MockEngine
            engines[name] = MockEngine(step_delay=config.mock_step_delay)
        elif name == "acestep":
            from .acestep import AceStepClient, AceStepEngine
            engines[name] = AceStepEngine(AceStepClient(config.acestep_url, config.acestep_api_key),
                                          timeout=config.acestep_timeout, lora_dir=config.lora_dir)
        else:  # load_config already validates names
            raise ValueError(f"unknown engine: {name}")
    return engines
