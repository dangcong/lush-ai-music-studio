"""AceStepEngine: adapter to the REST API server of ACE-Step 1.5 (`uv run acestep-api`, port 8001).

Ported from the research repo adapter (worker/backends/acestep.py), which was checked against the
ACE-Step source at commit ca1e85f. Only endpoints and fields used there are used here:
  - POST /release_task       -> data.task_id. JSON for text2music; multipart/form-data when the job
                                carries audio: file field `reference_audio` (global timbre / style
                                guide for text2music) or `src_audio` (task_type "cover").
  - POST /query_result       -> data[i].status (0 running / 1 done / 2 failed); data[i].result is a
                                JSON *string* (list of items: file, metas, seed_value, dit_model,
                                progress, stage, error); data[i].progress_text
  - GET  /v1/audio?path=...  -> audio file (`file` may already be a "/v1/audio?path=..." URL)
  - GET  /v1/model_inventory -> data.models[{name, is_default, is_loaded}], default_model
  - GET  /health             -> data.status == "ok"
  - /v1/lora/{status,load,scale,toggle}

The unit tests of this module use a fake HTTP server (respx). The request shapes for text2music,
reference audio and cover were also run by hand against the real server on render-node-01
(commit ca1e85f, acestep-v15-turbo, 2026-11-12).

Known limitation - cancellation: no cancel endpoint of ACE-Step has been verified, so a task that
was submitted cannot be stopped. When a cancel (or a lost lease) is detected mid-task, this engine
keeps polling until ACE-Step finishes the task, discards the result, and only then raises. The
worker therefore never claims the next job while the GPU is still busy with an orphaned task, and
never changes the shared LoRA state under a running task. The price is that a cancelled job is
reported as `cancelled` only after the current task ends (bounded by ACESTEP_TIMEOUT).
"""
from __future__ import annotations

import hashlib
import json
import logging
import re
import time
from pathlib import Path
from typing import Any, Callable, Optional

import httpx

from ..audio import probe_duration, probe_wav
from .base import (CAP_COVER, CAP_GENERATE, CAP_LORA, CAP_REFERENCE_AUDIO, MODE_COVER, MODE_REFERENCE,
                   STAGE_PREPARING, Engine, EngineError, EngineUnavailable, GenerateResult, JobContext,
                   input_path, job_mode)

log = logging.getLogger(__name__)

STATUS_RUNNING, STATUS_DONE, STATUS_FAILED = 0, 1, 2
MIN_DURATION, MAX_DURATION = 10.0, 600.0  # ACE-Step accepts audio_duration 10-600 seconds
TURBO_STEPS, BASE_STEPS = 8, 32  # default inference steps when the job does not set them
DEFAULT_GUIDANCE_SCALE = 7
LORA_KEY_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
LORA_CONFIG_FILE = "adapter_config.json"  # PEFT adapter directory marker


class AceStepError(EngineError):
    pass


class AceStepUnavailable(AceStepError, EngineUnavailable):
    """Cannot reach the ACE-Step API server (not started / wrong ACESTEP_URL) or it is not ready."""


class AceStepClient:
    """Thin client. Successful responses are wrapped as {data, code, error, timestamp, extra};
    error responses may only contain {detail}."""

    def __init__(self, base_url: str, api_key: str = "", timeout: float = 60.0,
                 client: Optional[httpx.Client] = None):
        self.base_url = base_url.rstrip("/")
        # Short connect timeout: fail fast when ACE-Step is down instead of hanging.
        self.http = client or httpx.Client(base_url=self.base_url,
                                           timeout=httpx.Timeout(timeout, connect=5.0))
        if api_key:
            self.http.headers["Authorization"] = f"Bearer {api_key}"

    def _request(self, method: str, url: str, **kw: Any) -> httpx.Response:
        try:
            return self.http.request(method, url, **kw)
        except httpx.TransportError as e:
            raise AceStepUnavailable(
                f"ACE-Step is not reachable at {self.base_url} ({type(e).__name__}). "
                "Start it with `uv run acestep-api` in the ACE-Step-1.5 directory.") from e

    @staticmethod
    def _unwrap(r: httpx.Response) -> Any:
        try:
            body = r.json()
        except ValueError:
            raise AceStepError(f"ACE-Step returned HTTP {r.status_code} (not JSON): {r.text[:300]}") from None
        if r.status_code >= 400 or not isinstance(body, dict):
            detail = body.get("detail") if isinstance(body, dict) else body
            raise AceStepError(f"ACE-Step HTTP {r.status_code}: {detail or body}")
        if "data" not in body and "detail" in body:
            raise AceStepError(str(body["detail"]))
        if body.get("error") or body.get("code", 200) != 200:
            raise AceStepError(f"ACE-Step code={body.get('code')} error={body.get('error')}")
        return body.get("data")

    def health(self) -> dict[str, Any]:
        return self._unwrap(self._request("GET", "/health", timeout=httpx.Timeout(10.0, connect=3.0))) or {}

    def model_inventory(self) -> dict[str, Any]:
        return self._unwrap(self._request("GET", "/v1/model_inventory")) or {}

    # ---------- LoRA ----------
    def lora_status(self) -> dict[str, Any]:
        return self._unwrap(self._request("GET", "/v1/lora/status")) or {}

    def lora_load(self, lora_path: str, adapter_name: str) -> Any:
        return self._unwrap(self._request("POST", "/v1/lora/load", timeout=300.0,
                                          json={"lora_path": lora_path, "adapter_name": adapter_name}))

    def lora_scale(self, scale: float, adapter_name: str) -> Any:
        return self._unwrap(self._request("POST", "/v1/lora/scale",
                                          json={"scale": scale, "adapter_name": adapter_name}))

    def lora_toggle(self, use_lora: bool) -> Any:
        return self._unwrap(self._request("POST", "/v1/lora/toggle", json={"use_lora": use_lora}))

    # /v1/lora/unload is deliberately not wrapped: at commit ca1e85f it does not actually remove the
    # LoRA layers (a "no LoRA" job would still run with the LoRA). Use lora_toggle(False) instead.

    # ---------- generation ----------
    def release_task(self, body: dict[str, Any], files: Optional[dict[str, Path]] = None) -> str:
        """Submit a task. With `files` ({form field: path}) the request is multipart/form-data, the
        only way the API accepts audio from a client (JSON paths must live in its own temp dir)."""
        if not files:
            data = self._unwrap(self._request("POST", "/release_task", json=body))
        else:
            form = {k: ("true" if v else "false") if isinstance(v, bool) else str(v)
                    for k, v in body.items() if v is not None}
            handles = {field: open(path, "rb") for field, path in files.items()}
            try:
                data = self._unwrap(self._request(
                    "POST", "/release_task", data=form, timeout=300.0,
                    files={field: (files[field].name, fh, "application/octet-stream")
                           for field, fh in handles.items()}))
            finally:
                for fh in handles.values():
                    fh.close()
        task_id = (data or {}).get("task_id")
        if not task_id:
            raise AceStepError(f"release_task returned no task_id: {data}")
        return str(task_id)

    def query_result(self, task_ids: list[str]) -> list[dict[str, Any]]:
        data = self._unwrap(self._request("POST", "/query_result", json={"task_id_list": task_ids}))
        return list(data or [])

    def download_audio(self, file_ref: str, dest: Path) -> Path:
        # `file` is either a ready URL "/v1/audio?path=..." or a raw path to pass as ?path=
        if file_ref.startswith(("http://", "https://", "/v1/audio")):
            url, params = file_ref, None
        else:
            url, params = "/v1/audio", {"path": file_ref}
        try:
            with self.http.stream("GET", url, params=params) as r:
                if r.status_code >= 400:
                    r.read()
                    raise AceStepError(f"audio download failed, HTTP {r.status_code}: {r.text[:300]}")
                with open(dest, "wb") as f:
                    for chunk in r.iter_bytes():
                        f.write(chunk)
        except httpx.TransportError as e:
            raise AceStepUnavailable(f"lost connection to ACE-Step while downloading audio: {e}") from e
        return dest


def parse_task_result(raw: Any) -> list[dict[str, Any]]:
    """The `result` field of /query_result is a JSON string (list of items)."""
    if raw in (None, ""):
        return []
    try:
        data = json.loads(raw) if isinstance(raw, str) else raw
    except ValueError:
        return []
    if isinstance(data, dict):
        data = [data]
    return [x for x in data if isinstance(x, dict)] if isinstance(data, list) else []


def first_seed(seed_value: Any) -> Optional[int]:
    """seed_value is a comma separated string of seeds."""
    if seed_value in (None, ""):
        return None
    try:
        return int(str(seed_value).split(",")[0].strip())
    except ValueError:
        return None


def normalize_time_signature(ts: Any) -> str:
    """The API takes 2/3/4/6 (for 2/4, 3/4, 4/4, 6/8)."""
    return str(ts).split("/")[0].strip() if ts else ""


def adapter_name(lora_key: str) -> str:
    """Stable ACE-Step adapter name for a LoRA key (identifier-safe, collision-free)."""
    safe = re.sub(r"[^A-Za-z0-9_]", "_", lora_key)
    return f"lush_lora_{safe}_{hashlib.sha1(lora_key.encode()).hexdigest()[:8]}"


def build_release_body(payload: dict[str, Any]) -> dict[str, Any]:
    """Map a protocol-v1 `generate` payload to the /release_task body.

    - mode `text` / `reference`: task_type "text2music" (the Reference Audio travels as a file);
    - mode `cover`: task_type "cover"; the length comes from the Source Audio, so no audio_duration.

    The v1 payload has no lyrics / vocal fields, so every job is submitted as instrumental, and no
    LM is used (thinking / CoT off), exactly like the research adapter's "no LM" path.
    """
    mode = job_mode(payload)
    try:
        model_key = str(payload["model"]["key"])
        prompt = str(payload["prompt"])
        seed = int(payload["seed"])
        duration = None if mode == MODE_COVER else float(payload["duration"])
    except (KeyError, TypeError, ValueError) as e:
        raise AceStepError(f"invalid generate payload: {e!r}") from e
    if seed < 0:
        raise AceStepError(f"seed must be chosen by the Control Plane (>= 0), got {seed}")
    if duration is not None and duration > MAX_DURATION:
        raise AceStepError(f"duration {duration:g}s exceeds the ACE-Step maximum of {MAX_DURATION:.0f}s")
    params = payload.get("params") or {}
    default_steps = TURBO_STEPS if "turbo" in model_key.lower() else BASE_STEPS
    try:
        steps = int(params.get("inference_steps") or default_steps)
        guidance = params.get("guidance_scale")
        guidance = DEFAULT_GUIDANCE_SCALE if guidance is None else float(guidance)
        bpm = int(payload["bpm"]) if payload.get("bpm") else None
        strength = params.get("audio_cover_strength")
        strength = None if strength is None else float(strength)
    except (TypeError, ValueError) as e:
        raise AceStepError(f"invalid generate params: {e!r}") from e
    if strength is not None and not 0.0 <= strength <= 1.0:
        raise AceStepError(f"audio_cover_strength must be between 0 and 1, got {strength:g}")

    body: dict[str, Any] = {
        "prompt": prompt,
        "lyrics": "[Instrumental]",
        "vocal_language": "unknown",
        "task_type": "cover" if mode == MODE_COVER else "text2music",
        "model": model_key,
        "thinking": False,
        "use_cot_caption": False,
        "use_cot_language": False,
        "use_random_seed": False,
        "seed": seed,
        "batch_size": 1,
        "inference_steps": steps,
        "audio_format": "wav",
        "guidance_scale": guidance,
        "lm_temperature": 0.85,
        "lm_negative_prompt": "NO USER INPUT",
    }
    if duration is not None:
        body["audio_duration"] = max(MIN_DURATION, duration)
    if mode == MODE_COVER and strength is not None:
        body["audio_cover_strength"] = strength
    if bpm:
        body["bpm"] = bpm
    if payload.get("key"):
        body["key_scale"] = str(payload["key"])
    ts = normalize_time_signature(payload.get("time_signature"))
    if ts:
        body["time_signature"] = ts
    return body


def build_release_files(payload: dict[str, Any]) -> dict[str, Path]:
    """Audio files to send with the task: {ACE-Step form field: local path}."""
    mode = job_mode(payload)
    if mode == MODE_REFERENCE:
        return {"reference_audio": input_path(payload, "reference_audio")}
    if mode == MODE_COVER:
        source = input_path(payload, "source_audio")
        length = probe_duration(source)
        if length is not None and length > MAX_DURATION:
            raise AceStepError(f"Source Audio is {length:.0f}s long; ACE-Step accepts at most "
                               f"{MAX_DURATION:.0f}s")
        return {"src_audio": source}
    return {}


class AceStepEngine(Engine):
    name = "acestep"
    version = "1.5"
    # Reference audio and cover are task types of every DiT variant, including turbo.
    capabilities = (CAP_GENERATE, CAP_LORA, CAP_REFERENCE_AUDIO, CAP_COVER)

    def __init__(self, client: AceStepClient, timeout: float = 1800.0, lora_dir: Optional[Path] = None,
                 poll_interval: float = 1.0, sleep: Callable[[float], None] = time.sleep):
        self.client = client
        self.timeout = timeout
        self.lora_dir = lora_dir
        self.poll_interval = poll_interval
        self._sleep = sleep

    # ---------- status / inventory ----------
    def health(self) -> Optional[str]:
        try:
            data = self.client.health()
        except AceStepError as e:
            return str(e)
        if data.get("status") != "ok":
            return f"ACE-Step reports an abnormal status: {data}"
        return None

    def list_models(self) -> list[dict[str, Any]]:
        """Only DiT models that are LOADED (the inventory also lists models that are merely on disk);
        if the server loads lazily and nothing is loaded yet, fall back to its default model."""
        data = self.client.model_inventory()
        loaded = [m["name"] for m in data.get("models") or [] if m.get("name") and m.get("is_loaded")]
        default = data.get("default_model")
        names = loaded or ([default] if default else [])
        return [{"engine": self.name, "key": n, "name": n, "capabilities": list(self.capabilities)}
                for n in names]

    def list_loras(self) -> list[str]:
        """LoRA keys = sub-directories of LORA_DIR that hold a PEFT adapter."""
        if self.lora_dir is None or not self.lora_dir.is_dir():
            return []
        return sorted(p.name for p in self.lora_dir.iterdir()
                      if p.is_dir() and LORA_KEY_RE.match(p.name) and (p / LORA_CONFIG_FILE).is_file())

    # ---------- LoRA ----------
    def _lora_path(self, key: str) -> Path:
        if not LORA_KEY_RE.match(key):
            raise AceStepError(f"invalid LoRA key {key!r}")
        if self.lora_dir is None:
            raise AceStepError("job needs a LoRA but LORA_DIR is not configured on this node")
        path = self.lora_dir / key
        if not (path / LORA_CONFIG_FILE).is_file():
            raise AceStepError(f"LoRA '{key}' not found on this node (expected {path / LORA_CONFIG_FILE})")
        return path.resolve()

    def _apply_lora_state(self, ctx: JobContext, payload: dict[str, Any]) -> dict[str, Any]:
        """LoRA is SHARED state of the model loaded in ACE-Step, so every job sets it explicitly.
        /v1/lora/unload is never used (broken, see AceStepClient). Instead each LoRA is loaded ONCE
        under its own adapter name; a job without LoRA disables the adapter layers (exact base-model
        output), a job with LoRA enables them, then selects its adapter and scale."""
        status = self.client.lora_status()
        loaded = bool(status.get("lora_loaded"))
        in_use = bool(status.get("use_lora"))
        adapters = [a if isinstance(a, str) else (a or {}).get("name") for a in status.get("adapters") or []]
        lora = payload.get("lora")
        if not lora:
            if loaded and in_use:
                self.client.lora_toggle(False)
                ctx.progress(0.03, "[acestep] disabled the loaded LoRA (this job uses none)", STAGE_PREPARING)
            return {}
        try:
            key = str(lora["key"])
            scale = min(1.0, max(0.0, float(lora.get("scale", 1.0))))  # the API only accepts 0.0-1.0
        except (KeyError, TypeError, ValueError) as e:
            raise AceStepError(f"invalid lora in payload: {e!r}") from e
        name = adapter_name(key)
        if name not in adapters:
            self.client.lora_load(str(self._lora_path(key)), adapter_name=name)
            ctx.progress(0.04, f"[acestep] loaded LoRA '{key}' as {name}", STAGE_PREPARING)
        self.client.lora_toggle(True)  # the scale call is ignored while LoRA is disabled -> enable first
        self.client.lora_scale(scale, adapter_name=name)  # also selects this adapter as the active one
        return {"lora_adapter": name, "lora_scale": scale}

    # ---------- generate ----------
    def generate(self, ctx: JobContext, payload: dict[str, Any]) -> GenerateResult:
        body = build_release_body(payload)  # parameter errors first: no engine state touched
        files = build_release_files(payload)
        problem = self.health()
        if problem:
            raise AceStepUnavailable(problem)
        ctx.raise_if_aborted()
        lora_info = self._apply_lora_state(ctx, payload)
        ctx.raise_if_aborted()  # last point where stopping is free: nothing is on the GPU yet

        task_id = self.client.release_task(body, files)
        length = f"{body['audio_duration']:.0f}s" if "audio_duration" in body else "from source audio"
        audio = "".join(f" {field}={path.name}" for field, path in files.items())
        ctx.progress(0.02, f"[acestep] task_id={task_id} task={body['task_type']} model={body['model']} "
                           f"seed={body['seed']} duration={length} steps={body['inference_steps']}{audio}",
                     STAGE_PREPARING)
        started = time.monotonic()
        try:
            item = self._wait_for_task(ctx, task_id, started)
        except EngineError:
            # The task is over (or unreachable) either way; an abort outranks the engine error.
            if ctx.cancel_requested or ctx.lease_lost:
                ctx.raise_if_aborted()
            raise
        ctx.raise_if_aborted()  # task finished but the job was aborted meanwhile: discard the result

        out = ctx.workdir / "master.wav"
        self.client.download_audio(item["file"], out)
        try:
            sample_rate, duration = probe_wav(out)
        except ValueError as e:
            raise AceStepError(f"ACE-Step returned an unreadable WAV file: {e}") from e
        took = time.monotonic() - started
        ctx.progress(0.97, f"[acestep] audio downloaded ({took:.1f}s)")
        seed = first_seed(item.get("seed_value"))
        return GenerateResult(
            audio_path=out,
            seed=seed if seed is not None else body["seed"],
            duration=duration,
            sample_rate=sample_rate,
            engine_info={
                "task_id": task_id,
                "task_type": body["task_type"],
                **({"audio_cover_strength": body["audio_cover_strength"]}
                   if "audio_cover_strength" in body else {}),
                "dit_model": item.get("dit_model") or body["model"],
                "inference_steps": body["inference_steps"],
                "guidance_scale": body["guidance_scale"],
                "metas": item.get("metas") or {},
                "elapsed_seconds": round(took, 1),
                **lora_info,
            },
        )

    def _wait_for_task(self, ctx: JobContext, task_id: str, started: float) -> dict[str, Any]:
        """Poll /query_result until the task is done. See the module docstring for abort handling."""
        last_text = ""
        draining = False
        while True:
            rows = self.client.query_result([task_id])
            row = next((r for r in rows if str(r.get("task_id")) == task_id), rows[0] if rows else {})
            status = int(row.get("status", STATUS_RUNNING))
            items = parse_task_result(row.get("result"))
            if status == STATUS_DONE:
                if not items or not items[0].get("file"):
                    raise AceStepError(f"task {task_id} finished without an audio file: {row.get('result')}")
                return items[0]
            if status == STATUS_FAILED:
                err = (items[0].get("error") if items else None) or row.get("progress_text") or row.get("result")
                raise AceStepError(f"ACE-Step task {task_id} failed: {str(err)[-1500:]}")
            if ctx.shutdown_requested:
                ctx.raise_if_aborted()  # worker is exiting: release the job now, nothing else will use the GPU
            if time.monotonic() - started > self.timeout:
                # Not retryable on purpose: the task may still be running inside ACE-Step, and an
                # automatic retry would queue the same long job behind it again.
                raise AceStepError(f"ACE-Step task {task_id} did not finish within {self.timeout:.0f}s "
                                   "(it may still be running inside ACE-Step)")
            if ctx.cancel_requested or ctx.lease_lost:
                if not draining:
                    draining = True
                    reason = "cancel requested" if ctx.cancel_requested else "lease lost"
                    log.warning("job %s: %s; ACE-Step has no cancel API, waiting for task %s to finish",
                                ctx.job_id, reason, task_id)
                    ctx.progress(0.05, f"[acestep] {reason}: waiting for the running task to finish "
                                       "(ACE-Step cannot cancel a task)")
            else:
                server_progress = float(items[0].get("progress") or 0) if items else 0.0
                text = str(row.get("progress_text") or (items[0].get("stage") if items else "") or "")
                ctx.progress(0.05 + 0.85 * min(1.0, max(0.0, server_progress)),
                             f"[acestep] {text}" if text and text != last_text else None)
                last_text = text or last_text
            self._sleep(self.poll_interval)
