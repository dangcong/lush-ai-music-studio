"""HTTP client for the Control Plane worker API (docs/WORKER_PROTOCOL.md, protocol version 1).

This is the only channel between the worker and the VPS. The worker always initiates requests.
Transport problems surface as `httpx.TransportError`; HTTP error responses as `ApiError`.
"""
from __future__ import annotations

from pathlib import Path
from typing import Any, Optional

import httpx

BASE_PATH = "/api/worker/v1"


class ApiError(Exception):
    """Non-2xx response. `code` is `error.code` from the body (e.g. `lease_lost`) when present."""

    def __init__(self, status: int, code: str = "", message: str = ""):
        super().__init__(f"HTTP {status} {code}: {message}".strip())
        self.status = status
        self.code = code
        self.message = message

    @property
    def lease_lost(self) -> bool:
        return self.status == 409 and self.code == "lease_lost"

    @property
    def auth_error(self) -> bool:
        return self.status in (401, 403)

    @property
    def transient(self) -> bool:
        return self.status >= 500


class ControlPlaneClient:
    def __init__(self, base_url: str, token: str, timeout: float = 30.0,
                 upload_timeout: float = 600.0, client: Optional[httpx.Client] = None):
        self.base_url = base_url.rstrip("/") + BASE_PATH
        self.upload_timeout = upload_timeout
        self.http = client or httpx.Client(timeout=httpx.Timeout(timeout, connect=10.0))
        self.http.headers["Authorization"] = f"Bearer {token}"

    def close(self) -> None:
        self.http.close()

    def _post(self, path: str, **kw: Any) -> httpx.Response:
        r = self.http.post(self.base_url + path, **kw)
        if r.status_code >= 400:
            code = message = ""
            try:
                err = r.json().get("error") or {}
                code, message = str(err.get("code") or ""), str(err.get("message") or "")
            except (ValueError, AttributeError):
                message = r.text[:300]
            raise ApiError(r.status_code, code, message)
        return r

    def register(self, body: dict[str, Any]) -> dict[str, Any]:
        return self._post("/register", json=body).json()

    def heartbeat(self, current_job_id: Optional[str], gpu: Optional[dict[str, Any]]) -> dict[str, Any]:
        return self._post("/heartbeat", json={"current_job_id": current_job_id, "gpu": gpu}).json()

    def claim(self) -> Optional[dict[str, Any]]:
        r = self._post("/jobs/claim", json={})
        return None if r.status_code == 204 else r.json()

    def progress(self, job_id: str, lease_token: str, progress: float, stage: str,
                 message: Optional[str] = None, metrics: Optional[dict[str, Any]] = None) -> dict[str, Any]:
        body: dict[str, Any] = {"lease_token": lease_token, "progress": max(0.0, min(1.0, progress)),
                                "stage": stage, "metrics": metrics or {}}
        if message:
            body["message"] = message
        return self._post(f"/jobs/{job_id}/progress", json=body).json()

    def upload_artifact(self, job_id: str, lease_token: str, kind: str, path: Path, sha256: str,
                        content_type: str) -> dict[str, Any]:
        with open(path, "rb") as f:
            return self._post(
                f"/jobs/{job_id}/artifacts",
                data={"lease_token": lease_token, "kind": kind, "sha256": sha256},
                files={"file": (path.name, f, content_type)},
                timeout=self.upload_timeout,
            ).json()

    def download_input(self, job_id: str, lease_token: str, artifact_id: str, dest: Path) -> Path:
        """Fetch one input file of a job we hold (Reference Audio / Source Audio)."""
        url = f"{self.base_url}/jobs/{job_id}/inputs/{artifact_id}"
        with self.http.stream("GET", url, params={"lease_token": lease_token},
                              timeout=self.upload_timeout) as r:
            if r.status_code >= 400:
                r.read()
                code = message = ""
                try:
                    err = r.json().get("error") or {}
                    code, message = str(err.get("code") or ""), str(err.get("message") or "")
                except (ValueError, AttributeError):
                    message = r.text[:300]
                raise ApiError(r.status_code, code, message)
            with open(dest, "wb") as f:
                for chunk in r.iter_bytes():
                    f.write(chunk)
        return dest

    def complete(self, job_id: str, lease_token: str, result: dict[str, Any]) -> None:
        self._post(f"/jobs/{job_id}/complete", json={"lease_token": lease_token, "result": result})

    def fail(self, job_id: str, lease_token: str, error: str, retryable: bool = False,
             cancelled: bool = False) -> None:
        self._post(f"/jobs/{job_id}/fail", json={"lease_token": lease_token, "error": error[:4000],
                                                 "retryable": retryable, "cancelled": cancelled})

    def release(self, job_id: str, lease_token: str) -> None:
        self._post(f"/jobs/{job_id}/release", json={"lease_token": lease_token})
