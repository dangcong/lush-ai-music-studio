from __future__ import annotations

import email
import email.policy
import threading
from pathlib import Path
from typing import Any, Optional

import httpx
import pytest

from lush_worker.api_client import ControlPlaneClient
from lush_worker.config import Config
from lush_worker.engines.base import JobCancelled, LeaseLost, ShutdownRequested
from lush_worker.engines.mock import MockEngine
from lush_worker.runner import Runner

CP = "http://cp.test"
BASE = f"{CP}/api/worker/v1"
TOKEN = "node-token"
JOB_ID = "11111111-1111-1111-1111-111111111111"
LEASE = "lease-abc"
REGISTER_OK = {"node_id": "n-1", "name": "render-node-01", "heartbeat_interval": 15,
               "lease_seconds": 90, "poll_interval": 3}


class FastEvent(threading.Event):
    """Event whose wait() never blocks, so retry/backoff loops run instantly in tests."""

    def wait(self, timeout: Optional[float] = None) -> bool:
        return self.is_set()


class FakeCtx:
    """JobContext for engine unit tests (no HTTP)."""

    def __init__(self, workdir: Path, job_id: str = JOB_ID):
        self.job_id = job_id
        self.workdir = workdir
        self.events: list[tuple[float, Optional[str], str]] = []
        self.cancel_requested = False
        self.lease_lost = False
        self.shutdown_requested = False
        self.on_progress = None  # optional hook(ctx) called after each progress report

    def progress(self, value, message=None, stage="running"):
        self.events.append((value, message, stage))
        if self.on_progress:
            self.on_progress(self)

    def raise_if_aborted(self):
        if self.lease_lost:
            raise LeaseLost("lease lost")
        if self.cancel_requested:
            raise JobCancelled("cancelled")
        if self.shutdown_requested:
            raise ShutdownRequested("shutdown")

    @property
    def messages(self) -> list[str]:
        return [m for _, m, _ in self.events if m]


def generate_payload(**kw: Any) -> dict[str, Any]:
    payload = {
        "prompt": "ambient piano, instrumental",
        "duration": 2,
        "seed": 123456,
        "bpm": 80, "key": "A minor", "time_signature": "4/4",
        "model": {"id": "m-1", "engine": "mock", "key": "mock-v1"},
        "lora": None,
        "params": {"inference_steps": None, "guidance_scale": None},
        "output": {"formats": ["wav", "mp3"]},
    }
    payload.update(kw)
    return payload


def make_job(**payload_kw: Any) -> dict[str, Any]:
    return {"id": JOB_ID, "type": "generate", "attempt": 1, "lease_token": LEASE,
            "lease_expires_at": "2026-01-01T00:00:00.000Z", "payload": generate_payload(**payload_kw)}


def parse_multipart(request: httpx.Request) -> dict[str, Any]:
    """Multipart body -> {field: str | bytes (for the file part)}, plus '_filename'."""
    raw = b"Content-Type: " + request.headers["content-type"].encode() + b"\r\n\r\n" + request.content
    msg = email.message_from_bytes(raw, policy=email.policy.HTTP)
    out: dict[str, Any] = {}
    for part in msg.iter_parts():
        name = part.get_param("name", header="content-disposition")
        data = part.get_payload(decode=True)
        if part.get_filename():
            out[name] = data
            out["_filename"] = part.get_filename()
        else:
            out[name] = data.decode()
    return out


@pytest.fixture
def config(tmp_path) -> Config:
    return Config(control_plane_url=CP, node_token=TOKEN, engines=("mock",), work_dir=tmp_path / "work",
                  mock_step_delay=0.0)


@pytest.fixture
def runner(config) -> Runner:
    r = Runner(config, {"mock": MockEngine(step_delay=0.0)}, ControlPlaneClient(CP, TOKEN),
               gpu_query=lambda: None)
    r._stop = FastEvent()
    r._retry_delay = 0.0
    return r
