"""Worker runner: register -> heartbeat thread -> claim loop -> run engine -> upload -> complete.

One job at a time (V1: one GPU job per node). See docs/WORKER_PROTOCOL.md.
"""
from __future__ import annotations

import logging
import shutil
import threading
import time
from pathlib import Path
from typing import Any, Callable, Optional

import httpx

from . import PROTOCOL_VERSION, __version__
from .api_client import ApiError, ControlPlaneClient
from .analysis import analysis_available, analyze_file
from .captioning import CaptionerSpec, run_caption_job
from .scoring import ScorerSpec
from .captioning import JOB_TYPE as CAPTION_JOB_TYPE
from .audio import make_mp3_preview, sha256_file
from .config import Config
from .engines.base import (MODES, STAGE_PREPARING, STAGE_RUNNING, Engine, EngineError, EngineUnavailable,
                           GenerateResult, JobAborted, JobCancelled, LeaseLost, ShutdownRequested, job_mode)
from .gpu import query_gpu
from . import lora_files
from .lora_files import JOB_TYPE as LORA_FILES_JOB_TYPE
from .training import JOB_TYPE as TRAIN_JOB_TYPE
from .training import SideStepTrainer, TrainingError

log = logging.getLogger(__name__)

JOB_TYPES = ["generate"]
ANALYZE_JOB_TYPE = "analyze_track"
ENGINE_PROGRESS_SHARE = 0.9  # engine progress 0..1 maps to job progress 0..0.9; the rest is upload
UPLOAD_ATTEMPTS = 4
FINISH_ATTEMPTS = 4  # complete / fail / release
REGISTER_MAX_DELAY = 30.0
AUTH_RETRY_DELAY = 30.0
ERROR_RETRY_DELAY = 5.0
REREGISTER_INTERVAL = 60.0  # refresh models / LoRAs / engine state on the Control Plane


class ProtocolMismatch(RuntimeError):
    """The Control Plane rejected our protocol version: retrying cannot help, upgrade the worker."""


class WorkerJobContext:
    """JobContext given to engines. Progress reporting never raises: a failed report is logged,
    a lost lease or a cancel request is recorded and surfaced through `raise_if_aborted()`."""

    def __init__(self, api: ControlPlaneClient, job: dict[str, Any], workdir: Path, stop: threading.Event):
        self.api = api
        self.job_id: str = str(job["id"])
        self.lease_token: str = job["lease_token"]
        self.workdir = workdir
        self._stop = stop
        self._cancel = threading.Event()
        self._lease_lost = threading.Event()

    @property
    def cancel_requested(self) -> bool:
        return self._cancel.is_set()

    @property
    def lease_lost(self) -> bool:
        return self._lease_lost.is_set()

    @property
    def shutdown_requested(self) -> bool:
        return self._stop.is_set()

    def request_cancel(self) -> None:
        self._cancel.set()

    def mark_lease_lost(self) -> None:
        self._lease_lost.set()

    def raise_if_aborted(self) -> None:
        if self.lease_lost:
            raise LeaseLost(f"job {self.job_id}: lease lost")
        if self.cancel_requested:
            raise JobCancelled(f"job {self.job_id}: cancelled")
        if self.shutdown_requested:
            raise ShutdownRequested(f"job {self.job_id}: worker shutting down")

    def progress(self, value: float, message: Optional[str] = None, stage: str = STAGE_RUNNING) -> None:
        """Engine-facing: `value` is the engine's own 0..1 progress."""
        self.report(max(0.0, min(1.0, value)) * ENGINE_PROGRESS_SHARE, message, stage)

    def report(self, value: float, message: Optional[str] = None, stage: str = STAGE_RUNNING,
               phase: Optional[str] = None, metrics: Optional[list[dict[str, Any]]] = None) -> None:
        if message:
            log.info("job %s %3.0f%% %s", self.job_id, value * 100, message)
        if self.lease_lost:
            return
        try:
            res = self.api.progress(self.job_id, self.lease_token, value, stage, message, phase, metrics)
            if isinstance(res, dict) and res.get("cancel_requested"):
                self.request_cancel()
        except ApiError as e:
            if e.lease_lost:
                log.warning("job %s: lease lost (409), the job will be dropped", self.job_id)
                self.mark_lease_lost()
            else:
                log.warning("job %s: progress report rejected: %s", self.job_id, e)
        except Exception as e:  # reporting progress must never break the job
            log.warning("job %s: could not report progress: %s", self.job_id, e)


class Runner:
    def __init__(self, config: Config, engines: dict[str, Engine], api: ControlPlaneClient,
                 gpu_query: Callable[[], Optional[dict[str, Any]]] = query_gpu,
                 trainer: Optional[SideStepTrainer] = None,
                 captioners: Optional[list[CaptionerSpec]] = None,
                 scorers: Optional[dict[str, ScorerSpec]] = None):
        self.config = config
        self.engines = engines
        self.api = api
        self.trainer = trainer
        self.captioners = list(captioners or [])
        self.scorers = dict(scorers or {})
        self.caption_timeout = 600.0
        self._gpu_query = gpu_query
        self._stop = threading.Event()
        self._current: Optional[WorkerJobContext] = None
        self.heartbeat_interval = 15.0
        self.poll_interval = 3.0
        self.lease_seconds = 90.0
        self._retry_delay = 1.0  # base delay between upload / finish retries

    # ---------- lifecycle ----------
    def stop(self) -> None:
        """Request a controlled shutdown (signal handlers call this)."""
        self._stop.set()

    @property
    def stopping(self) -> bool:
        return self._stop.is_set()

    # ---------- register ----------
    def registration(self, fresh_start: bool) -> dict[str, Any]:
        models: list[dict[str, Any]] = []
        loras: set[str] = set()
        for engine in self.engines.values():
            try:
                models.extend(engine.list_models())
                loras.update(engine.list_loras())
            except Exception as e:
                # Report no models for an unreachable engine: the Control Plane then keeps its jobs
                # queued instead of handing them to us to fail. Re-registration restores them.
                log.warning("engine %s: cannot list models (%s); reporting none for now", engine.name, e)
        models.extend(c.describe() for c in self.captioners)
        models.extend(s.describe() for s in self.scorers.values())
        gpu = self._gpu_query()
        body: dict[str, Any] = {
            "protocol_version": PROTOCOL_VERSION,
            "worker_version": __version__,
            "fresh_start": fresh_start,
            "job_types": (list(JOB_TYPES) + ([TRAIN_JOB_TYPE, LORA_FILES_JOB_TYPE] if self.trainer else [])
                          + ([ANALYZE_JOB_TYPE] if analysis_available() else [])
                          + ([CAPTION_JOB_TYPE] if self.captioners else [])),
            "engines": [e.describe() for e in self.engines.values()],
            "models": models,
            "loras": sorted(loras),
            "gpu": {"name": gpu["name"], "vram_total_mb": gpu["vram_total_mb"]} if gpu else None,
        }
        if self.trainer:
            body["datasets"] = self.trainer.list_datasets()
        return body

    def register(self, fresh_start: bool) -> dict[str, Any]:
        body = self.registration(fresh_start)
        res = self.api.register(body)
        self.heartbeat_interval = float(res.get("heartbeat_interval") or self.heartbeat_interval)
        self.poll_interval = float(res.get("poll_interval") or self.poll_interval)
        self.lease_seconds = float(res.get("lease_seconds") or self.lease_seconds)
        log.info("registered as node %s (%s): engines=%s models=%s loras=%s",
                 res.get("name"), res.get("node_id"), list(self.engines),
                 [m["key"] for m in body["models"]], body["loras"])
        return res

    def register_with_retry(self, fresh_start: bool = True) -> bool:
        """Register, retrying with exponential backoff. Returns False if stopped before success.
        Raises ProtocolMismatch on `409 protocol_mismatch`."""
        delay = 1.0
        while not self._stop.is_set():
            try:
                self.register(fresh_start)
                return True
            except ApiError as e:
                if e.status == 409 and e.code == "protocol_mismatch":
                    raise ProtocolMismatch(
                        f"Control Plane does not support protocol version {PROTOCOL_VERSION}: {e.message}") from e
                if e.auth_error:
                    log.error("register rejected (%s): check NODE_TOKEN / whether the node is disabled. "
                              "Retrying in %.0fs", e, delay)
                else:
                    log.warning("register failed (%s). Retrying in %.0fs", e, delay)
            except (httpx.HTTPError, ValueError) as e:
                log.warning("cannot reach the Control Plane (%s: %s). Retrying in %.0fs",
                            type(e).__name__, e, delay)
            self._stop.wait(delay)
            delay = min(delay * 2, REGISTER_MAX_DELAY)
        return False

    # ---------- heartbeat ----------
    def heartbeat_once(self) -> None:
        ctx = self._current
        gpu = self._gpu_query()
        stats = ({"utilization": gpu["utilization"], "vram_used_mb": gpu["vram_used_mb"],
                  "vram_total_mb": gpu["vram_total_mb"]} if gpu else None)
        res = self.api.heartbeat(ctx.job_id if ctx else None, stats)
        cancel_ids = {str(j) for j in res.get("cancel_job_ids") or []}
        if ctx is not None and ctx.job_id in cancel_ids and not ctx.cancel_requested:
            log.info("job %s: cancel requested by the Control Plane", ctx.job_id)
            ctx.request_cancel()

    def _heartbeat_loop(self) -> None:
        last_register = time.monotonic()
        while not self._stop.wait(self.heartbeat_interval):
            try:
                self.heartbeat_once()
                if time.monotonic() - last_register >= REREGISTER_INTERVAL:
                    last_register = time.monotonic()
                    self.register(fresh_start=False)
            except ApiError as e:
                if e.auth_error:
                    log.error("heartbeat rejected (%s): check NODE_TOKEN / whether the node is disabled", e)
                else:
                    log.warning("heartbeat failed: %s", e)
            except Exception as e:
                log.warning("heartbeat failed: %s: %s", type(e).__name__, e)

    # ---------- job ----------
    def _call_with_retry(self, what: str, fn: Callable[[], Any], attempts: int,
                         retry_checksum: bool = False) -> Any:
        """Call a Control Plane endpoint, retrying network errors, 5xx and (for uploads)
        `422 checksum_mismatch`. `409 lease_lost` becomes LeaseLost."""
        for attempt in range(1, attempts + 1):
            try:
                return fn()
            except ApiError as e:
                if e.lease_lost:
                    raise LeaseLost(f"{what}: lease lost") from e
                retry = e.transient or (retry_checksum and e.status == 422 and e.code == "checksum_mismatch")
                if not retry or attempt == attempts:
                    raise
                log.warning("%s failed (%s), attempt %d/%d", what, e, attempt, attempts)
            except httpx.TransportError as e:
                if attempt == attempts:
                    raise
                log.warning("%s failed (%s: %s), attempt %d/%d", what, type(e).__name__, e, attempt, attempts)
            time.sleep(self._retry_delay * attempt)

    def _upload(self, ctx: WorkerJobContext, kind: str, path: Path, content_type: str) -> None:
        digest = sha256_file(path)
        self._call_with_retry(
            f"job {ctx.job_id}: upload {kind}",
            lambda: self.api.upload_artifact(ctx.job_id, ctx.lease_token, kind, path, digest, content_type),
            UPLOAD_ATTEMPTS, retry_checksum=True)
        log.info("job %s: uploaded %s (%d bytes, sha256=%s)", ctx.job_id, kind, path.stat().st_size, digest)

    def _fail(self, ctx: WorkerJobContext, error: str, retryable: bool = False, cancelled: bool = False) -> None:
        try:
            self._call_with_retry(f"job {ctx.job_id}: fail",
                                  lambda: self.api.fail(ctx.job_id, ctx.lease_token, error, retryable, cancelled),
                                  FINISH_ATTEMPTS)
        except LeaseLost:
            log.warning("job %s: lease already lost, nothing to report", ctx.job_id)
        except Exception as e:
            # The lease will expire and the Control Plane reaper will requeue / fail the job.
            log.error("job %s: could not report failure (%s); leaving it to the lease reaper", ctx.job_id, e)

    def _release(self, ctx: WorkerJobContext) -> None:
        try:
            self.api.release(ctx.job_id, ctx.lease_token)
            log.info("job %s: released back to the queue", ctx.job_id)
        except Exception as e:
            log.warning("job %s: could not release (%s); the lease will expire instead", ctx.job_id, e)

    @staticmethod
    def _raise_if_cancelled_or_lost(ctx: WorkerJobContext) -> None:
        """Abort check used once the audio exists: a pending shutdown does not stop the (short)
        upload + complete, which is cheaper than releasing the job and generating it again."""
        if ctx.lease_lost:
            raise LeaseLost(f"job {ctx.job_id}: lease lost")
        if ctx.cancel_requested:
            raise JobCancelled(f"job {ctx.job_id}: cancelled")

    def _fetch_inputs(self, ctx: WorkerJobContext, payload: dict[str, Any]) -> dict[str, Any]:
        """Download the job's input audio into the workdir and return a payload whose inputs carry
        the local `path`. A file that does not match its checksum is never handed to the engine."""
        inputs = payload.get("inputs") or {}
        if not inputs:
            return payload
        local: dict[str, Any] = {}
        for role, info in inputs.items():
            try:
                artifact_id, expected = str(info["artifact_id"]), str(info["sha256"]).lower()
            except (KeyError, TypeError) as e:
                raise EngineError(f"invalid input '{role}' in payload: {e!r}") from e
            suffix = Path(str(info.get("filename") or "")).suffix.lower()
            dest = ctx.workdir / f"input_{role}{suffix if suffix in ('.wav', '.mp3', '.flac') else ''}"
            ctx.report(0.0, f"downloading {role} ({info.get('filename')})", stage=STAGE_PREPARING)
            self._call_with_retry(
                f"job {ctx.job_id}: download {role}",
                lambda: self.api.download_input(ctx.job_id, ctx.lease_token, artifact_id, dest),
                UPLOAD_ATTEMPTS)
            digest = sha256_file(dest)
            if digest != expected:
                raise EngineUnavailable(f"input '{role}' is corrupted (sha256 {digest}, expected {expected})")
            local[role] = {**info, "path": str(dest)}
        return {**payload, "inputs": local}

    def _execute_training(self, ctx: WorkerJobContext, job: dict[str, Any]) -> None:
        if self.trainer is None:
            raise EngineError("this node is not configured for training")
        ctx.report(0.0, f"claimed by worker {__version__}, trainer=sidestep attempt={job.get('attempt')}",
                   stage="preparing")
        ctx.raise_if_aborted()
        try:
            payload = job.get("payload") or {}
            if payload.get("samples"):
                payload = self._fetch_inputs(ctx, payload)
            result = self.trainer.run(ctx, payload)
        except ShutdownRequested as e:
            # Never hand a training run back to the queue: it would silently start over.
            raise TrainingError("the worker was stopped during training; the run was not finished") from e
        self._call_with_retry(f"job {ctx.job_id}: complete",
                              lambda: self.api.complete(ctx.job_id, ctx.lease_token, result), FINISH_ATTEMPTS)
        log.info("job %s: training completed (%s epochs, %s optimizer steps, stop_reason=%s)", ctx.job_id,
                 result.get("epochs_completed"), result.get("optimizer_steps"), result.get("stop_reason"))

    def _execute_analysis(self, ctx: WorkerJobContext, job: dict[str, Any]) -> None:
        ctx.report(0.0, f"claimed by worker {__version__}, analyze_track attempt={job.get('attempt')}",
                   stage="preparing")
        ctx.raise_if_aborted()
        payload = self._fetch_inputs(ctx, job.get("payload") or {})
        track = (payload.get("inputs") or {}).get("track")
        if not track:
            raise EngineError("analyze_track job has no 'track' input")
        ctx.raise_if_aborted()
        ctx.report(0.3, "analyzing tempo, key and loudness")
        try:
            result = analyze_file(Path(track["path"]))
        except Exception as e:
            raise EngineError(f"could not analyze the track: {e}") from e
        self._raise_if_cancelled_or_lost(ctx)
        self._call_with_retry(f"job {ctx.job_id}: complete",
                              lambda: self.api.complete(ctx.job_id, ctx.lease_token, result), FINISH_ATTEMPTS)
        log.info("job %s: analysis completed (bpm=%s key=%s)", ctx.job_id,
                 result["tempo"]["bpm"], result["key"]["key"])

    def _execute_captioning(self, ctx: WorkerJobContext, job: dict[str, Any]) -> None:
        payload = job.get("payload") or {}
        key = (payload.get("model") or {}).get("key")
        spec = next((c for c in self.captioners if c.key == key), None)
        if spec is None:
            raise EngineError(f"Captioner {key} không có trên máy này")
        ctx.report(0.0, f"claimed by worker {__version__}, captioner={spec.key}@{spec.version} "
                        f"attempt={job.get('attempt')}", stage="preparing")
        ctx.raise_if_aborted()
        payload = self._fetch_inputs(ctx, payload)
        ctx.raise_if_aborted()
        result = run_caption_job(ctx, self.api, spec, payload, track_timeout=self.caption_timeout)
        self._raise_if_cancelled_or_lost(ctx)
        self._call_with_retry(f"job {ctx.job_id}: complete",
                              lambda: self.api.complete(ctx.job_id, ctx.lease_token, result), FINISH_ATTEMPTS)
        log.info("job %s: captioning completed (done=%s failed=%s)", ctx.job_id, result["done"],
                 len(result["failed"]))

    def _execute_lora_files(self, ctx: WorkerJobContext, job: dict[str, Any]) -> None:
        if self.trainer is None:
            raise EngineError("this node is not configured for training")
        ctx.report(0.0, f"claimed by worker {__version__}, lora_files attempt={job.get('attempt')}",
                   stage="preparing")
        ctx.raise_if_aborted()
        result = lora_files.run(job.get("payload") or {}, ctx, checkpoint_root=self.trainer.training_dir,
                                lora_dir=self.trainer.lora_dir)
        self._raise_if_cancelled_or_lost(ctx)
        self._call_with_retry(f"job {ctx.job_id}: complete",
                              lambda: self.api.complete(ctx.job_id, ctx.lease_token, result), FINISH_ATTEMPTS)
        log.info("job %s: lora_files %s completed", ctx.job_id, (job.get("payload") or {}).get("op"))
        self._refresh_registration()  # a published LoRA must show up in `loras` right away

    def _refresh_registration(self) -> None:
        """Report models / LoRAs again right away (inference was restarted, a LoRA may be new)."""
        try:
            self.register(fresh_start=False)
        except Exception as e:
            log.warning("could not refresh the registration: %s", e)

    def _execute(self, ctx: WorkerJobContext, job: dict[str, Any]) -> None:
        payload = job.get("payload") or {}
        if job.get("type") == TRAIN_JOB_TYPE:
            try:
                self._execute_training(ctx, job)
            finally:
                self._current = None  # the registration below must not look like a heartbeat for this job
                self._refresh_registration()
            return
        if job.get("type") == LORA_FILES_JOB_TYPE:
            self._execute_lora_files(ctx, job)
            return
        if job.get("type") == ANALYZE_JOB_TYPE:
            self._execute_analysis(ctx, job)
            return
        if job.get("type") == CAPTION_JOB_TYPE:
            self._execute_captioning(ctx, job)
            return
        if job.get("type") != "generate":
            raise EngineError(f"unsupported job type: {job.get('type')!r}")
        engine_name = (payload.get("model") or {}).get("engine")
        engine = self.engines.get(engine_name)
        if engine is None:
            raise EngineError(f"engine {engine_name!r} is not enabled on this node (enabled: {list(self.engines)})")
        mode = job_mode(payload)
        capability, role = MODES[mode]
        if "generate" not in engine.capabilities or capability not in engine.capabilities:
            # Never fall back to plain text-to-music: the user asked for something else.
            raise EngineError(f"engine {engine.name!r} does not support '{capability}' (mode {mode!r})")
        if role and role not in (payload.get("inputs") or {}):
            raise EngineError(f"mode {mode!r} needs a '{role}' input, the job has none")

        ctx.report(0.0, f"claimed by worker {__version__}, engine={engine.name} attempt={job.get('attempt')}",
                   stage="preparing")
        ctx.raise_if_aborted()
        payload = self._fetch_inputs(ctx, payload)
        ctx.raise_if_aborted()
        result: GenerateResult = engine.generate(ctx, payload)
        self._raise_if_cancelled_or_lost(ctx)

        preview: Optional[Path] = None
        formats = (payload.get("output") or {}).get("formats") or ["wav", "mp3"]
        if "mp3" in formats:
            preview = make_mp3_preview(result.audio_path, ctx.workdir / "preview.mp3")
            if preview is None:
                ctx.report(ENGINE_PROGRESS_SHARE, "MP3 preview skipped (ffmpeg unavailable or failed)")
        self._raise_if_cancelled_or_lost(ctx)

        ctx.report(0.92, "uploading audio_master")
        self._upload(ctx, "audio_master", result.audio_path, "audio/wav")
        if preview is not None:
            ctx.report(0.96, "uploading audio_preview")
            self._upload(ctx, "audio_preview", preview, "audio/mpeg")
        self._raise_if_cancelled_or_lost(ctx)

        self._call_with_retry(f"job {ctx.job_id}: complete", lambda: self.api.complete(ctx.job_id, ctx.lease_token, {
            "seed": result.seed, "duration": result.duration, "sample_rate": result.sample_rate,
            "engine": engine.name, "model_key": (payload.get("model") or {}).get("key"),
            "engine_info": result.engine_info,
        }), FINISH_ATTEMPTS)
        log.info("job %s: completed (seed=%s duration=%.1fs)", ctx.job_id, result.seed, result.duration)

    def run_job(self, job: dict[str, Any]) -> None:
        job_id = str(job["id"])
        workdir = self.config.work_dir / "jobs" / job_id
        shutil.rmtree(workdir, ignore_errors=True)
        workdir.mkdir(parents=True, exist_ok=True)
        ctx = WorkerJobContext(self.api, job, workdir, self._stop)
        self._current = ctx
        log.info("job %s: claimed (type=%s attempt=%s)", job_id, job.get("type"), job.get("attempt"))
        try:
            self._execute(ctx, job)
        except LeaseLost:
            log.warning("job %s: dropped, the lease was lost (nothing uploaded from here on)", job_id)
        except JobCancelled:
            log.info("job %s: cancelled", job_id)
            self._fail(ctx, "cancelled on request", cancelled=True)
        except ShutdownRequested:
            self._release(ctx)
        except KeyboardInterrupt:
            self._stop.set()
            self._release(ctx)
        except JobAborted as e:  # future abort reasons: treat as a plain failure
            self._fail(ctx, str(e))
        except EngineError as e:
            log.error("job %s: engine error (retryable=%s): %s", job_id, e.retryable, e)
            self._fail(ctx, str(e), retryable=e.retryable)
        except httpx.TransportError as e:
            log.error("job %s: lost the Control Plane while finishing: %s", job_id, e)
            self._fail(ctx, f"connection to the Control Plane failed: {type(e).__name__}: {e}", retryable=True)
        except ApiError as e:
            log.error("job %s: Control Plane rejected the result: %s", job_id, e)
            self._fail(ctx, f"Control Plane rejected the result: {e}", retryable=e.transient)
        except Exception as e:
            log.exception("job %s: unexpected error", job_id)
            self._fail(ctx, f"{type(e).__name__}: {e}")
        finally:
            self._current = None
            shutil.rmtree(workdir, ignore_errors=True)

    def run_once(self) -> bool:
        """Claim and run at most one job. Returns True if a job was claimed."""
        job = self.api.claim()
        if job is None:
            return False
        self.run_job(job)
        return True

    # ---------- main loop ----------
    def run_forever(self) -> None:
        if self.trainer:
            self.trainer.recover()
        if not self.register_with_retry(fresh_start=True):
            return
        threading.Thread(target=self._heartbeat_loop, name="heartbeat", daemon=True).start()
        log.info("polling %s every %.1fs", self.api.base_url, self.poll_interval)
        try:
            while not self._stop.is_set():
                try:
                    if not self.run_once():
                        self._stop.wait(self.poll_interval)
                except ApiError as e:
                    if e.auth_error:
                        log.error("claim rejected (%s): check NODE_TOKEN / whether the node is disabled. "
                                  "Retrying in %.0fs", e, AUTH_RETRY_DELAY)
                        self._stop.wait(AUTH_RETRY_DELAY)
                    else:
                        log.warning("claim failed: %s", e)
                        self._stop.wait(ERROR_RETRY_DELAY)
                except (httpx.HTTPError, ValueError) as e:
                    log.warning("lost the Control Plane (%s: %s); re-registering", type(e).__name__, e)
                    self._stop.wait(ERROR_RETRY_DELAY)
                    # We hold no job here, so fresh_start=True is accurate: it lets the Control Plane
                    # requeue anything it still believes this node is running.
                    self.register_with_retry(fresh_start=True)
        except KeyboardInterrupt:
            pass
        finally:
            self._stop.set()
            log.info("worker stopped")
