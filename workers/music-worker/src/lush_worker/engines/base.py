"""Engine adapter abstraction.

An engine only knows a `JobContext` (progress reporting, abort flags, scratch directory) and the job
payload; it knows nothing about the Control Plane HTTP API. Engines must not assume that any other
engine supports the same capabilities: each one declares its own `capabilities`.
"""
from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Optional, Protocol

# Capability names used so far. Later phases may add: extend, inpaint.
CAP_GENERATE = "generate"
CAP_LORA = "lora"
CAP_REFERENCE_AUDIO = "reference_audio"  # text + Reference Audio (global timbre / style guide)
CAP_COVER = "cover"  # Source Audio restyled by the prompt (keeps its structure)

MODE_TEXT, MODE_REFERENCE, MODE_COVER = "text", "reference", "cover"
# Generation mode -> (capability the engine must declare, input role the payload must carry).
MODES: dict[str, tuple[str, Optional[str]]] = {
    MODE_TEXT: (CAP_GENERATE, None),
    MODE_REFERENCE: (CAP_REFERENCE_AUDIO, "reference_audio"),
    MODE_COVER: (CAP_COVER, "source_audio"),
}

STAGE_PREPARING = "preparing"
STAGE_RUNNING = "running"


class EngineError(RuntimeError):
    """Job failed inside the engine. `retryable=False`: retrying the same job cannot help
    (bad parameters, engine rejected the task)."""

    retryable = False


class EngineUnavailable(EngineError):
    """The engine is temporarily not usable (server down, timeout): the job may be retried."""

    retryable = True


def job_mode(payload: dict[str, Any]) -> str:
    """Generation mode of a `generate` payload (payloads from before the modes existed are text)."""
    mode = payload.get("mode") or MODE_TEXT
    if mode not in MODES:
        raise EngineError(f"unknown generation mode {mode!r}")
    return mode


def input_path(payload: dict[str, Any], role: str) -> Path:
    """Local file of an input the runner downloaded (`payload.inputs.<role>.path`)."""
    path = ((payload.get("inputs") or {}).get(role) or {}).get("path")
    if not path or not Path(path).is_file():
        raise EngineError(f"job has no downloaded '{role}' input")
    return Path(path)


class JobAborted(Exception):
    """Base for the reasons a job must stop before producing a result."""


class JobCancelled(JobAborted):
    """The Control Plane asked to cancel this job."""


class LeaseLost(JobAborted):
    """The job no longer belongs to this node: drop it, upload nothing."""


class ShutdownRequested(JobAborted):
    """The worker is shutting down: the job is released back to the queue."""


class JobContext(Protocol):
    job_id: str
    workdir: Path

    def progress(self, value: float, message: Optional[str] = None, stage: str = STAGE_RUNNING) -> None:
        """Report progress (0..1 of the engine's own work). Never raises."""
        ...

    @property
    def cancel_requested(self) -> bool: ...

    @property
    def lease_lost(self) -> bool: ...

    @property
    def shutdown_requested(self) -> bool: ...

    def raise_if_aborted(self) -> None:
        """Raise the matching JobAborted subclass if the job must stop. Engines call this between
        steps, at points where stopping is safe."""
        ...


@dataclass
class GenerateResult:
    audio_path: Path  # WAV file inside ctx.workdir
    seed: int
    duration: float  # seconds, measured from the produced file
    sample_rate: int
    engine_info: dict[str, Any] = field(default_factory=dict)


class Engine(ABC):
    name: str = "base"
    version: str = "0"
    capabilities: tuple[str, ...] = ()

    @abstractmethod
    def list_models(self) -> list[dict[str, Any]]:
        """Models usable right now: [{"engine", "key", "name", "capabilities"}].
        May raise EngineError when the engine cannot be reached."""

    def list_loras(self) -> list[str]:
        """Keys of LoRAs available on this node for this engine."""
        return []

    def health(self) -> Optional[str]:
        """None when the engine is ready, otherwise a short human-readable problem."""
        return None

    @abstractmethod
    def generate(self, ctx: JobContext, payload: dict[str, Any]) -> GenerateResult:
        """Run a `generate` job. Raises EngineError / EngineUnavailable / JobAborted."""

    def describe(self) -> dict[str, Any]:
        return {"name": self.name, "version": self.version, "capabilities": list(self.capabilities)}
