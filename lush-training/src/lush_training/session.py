"""One training session: the loop around a backend, with limits, safe stop, checkpoints and resume.

This module owns everything that does not depend on the model: when to stop, when to checkpoint,
what to record, what never to do (restart from step 0 silently, checkpoint after a NaN). The backend
owns loss, backward and the weight update. The lifecycle here is tested without a GPU with a small
deterministic backend; that says nothing about whether real training fits a GPU.
"""
from __future__ import annotations

import os
import signal
import time
from pathlib import Path
from typing import Any, Callable, Optional, Protocol

from . import checkpoint
from .run import event, load_run, trainer_pid, write_progress_report, write_status
from .util import LushTrainingError, append_jsonl, atomic_write_bytes, read_json

# Exit codes of the trainer process. A caller reads these instead of guessing from the log.
EXIT_COMPLETED = 0        # reached max_steps / max_epochs / early stopping
EXIT_CRASH = 1
EXIT_TIME_LIMIT = 10      # session or total time budget, or --stop-at: checkpoint saved, resumable
EXIT_STOP_REQUESTED = 11  # STOP file or SIGTERM/SIGINT: checkpoint saved, resumable
EXIT_OOM = 20
EXIT_NON_FINITE = 21
EXIT_NO_VALID_CHECKPOINT = 22
EXIT_MISMATCH = 23        # dataset / parent checkpoint / config no longer match run.json
EXIT_ALREADY_RUNNING = 24
RESUMABLE_REASONS = ("session_time_limit", "stop_at", "stop_requested")
MIN_STEPS_FOR_ETA = 20
REPORT_EVERY_SECONDS = 60


class NonFiniteLoss(Exception):
    """The loss is NaN or infinite. No checkpoint is written after this."""


class OutOfMemory(Exception):
    """The GPU ran out of memory. The configuration is never changed automatically."""


class Mismatch(Exception):
    """Data on disk no longer matches what run.json recorded."""


class Backend(Protocol):
    weight_files: tuple[str, ...]

    def setup(self) -> dict[str, Any]:
        """Load everything; return facts for the session record (parameter counts, dependencies)."""

    def steps_per_epoch(self) -> int: ...

    def train_step(self, step: int) -> dict[str, Any]:
        """Run optimizer step number `step` (1-based). Returns at least {"loss": float, "lr": float}."""

    def validate(self) -> Optional[float]: ...

    def save_state(self, directory: Path) -> None: ...

    def load_state(self, directory: Path) -> None: ...


class StopFlag:
    def __init__(self) -> None:
        self.signal: Optional[str] = None

    def install(self) -> None:
        def handler(signum: int, _frame: Any) -> None:
            self.signal = signal.Signals(signum).name
        for sig in (signal.SIGINT, signal.SIGTERM):
            try:
                signal.signal(sig, handler)
            except (ValueError, OSError):  # not the main thread, or unsupported on this platform
                pass


def run_session(run_dir: Path, backend: Backend, *, session_hours: Optional[float] = None,
                stop_at: Optional[float] = None, clock: Callable[[], float] = time.time,
                install_signals: bool = True) -> int:
    """Train until the first limit is reached. Returns one of the EXIT_* codes."""
    run_dir = Path(run_dir)
    info = load_run(run_dir)
    cfg = info["config"]
    limits, ck, val = cfg["limits"], cfg["checkpoint"], cfg["validation"]

    if trainer_pid(run_dir) not in (None, os.getpid()):
        return EXIT_ALREADY_RUNNING
    atomic_write_bytes(run_dir / "trainer.pid", f"{os.getpid()}\n".encode())
    stop_file = run_dir / "STOP"
    if stop_file.exists():
        stop_file.unlink()  # a stop request belongs to the session it was sent to
    flag = StopFlag()
    if install_signals:
        flag.install()
    session_started = clock()
    progress: dict[str, Any] = {"step": 0, "train_seconds": 0.0, "sessions": 0, "best_validation": None,
                                "validations_without_improvement": 0}
    status: dict[str, Any] = {"state": "running", "pid": os.getpid(), "step": 0, "max_steps": limits["max_steps"]}

    def report() -> None:
        try:  # the report is for people; a problem writing it must never stop training
            write_progress_report(run_dir)
        except Exception as e:
            event(run_dir, "progress_report_failed", error=f"{type(e).__name__}: {e}")

    def finish(code: int, state: str, reason: str, **extra: Any) -> int:
        status.update({"state": state, "stop_reason": reason, **extra})
        status.pop("pid", None)
        write_status(run_dir, status)
        event(run_dir, "session_end", exit_code=code, state=state, stop_reason=reason, step=progress["step"], **extra)
        report()
        try:
            (run_dir / "trainer.pid").unlink()
        except OSError:
            pass
        return code

    try:
        removed = checkpoint.remove_partials(run_dir)
        if removed:
            event(run_dir, "partial_checkpoints_removed", names=removed)
        facts = backend.setup()
        latest, rejected = checkpoint.latest_valid(run_dir)
        for r in rejected:
            event(run_dir, "checkpoint_rejected", **r)
        if latest is None and checkpoint.list_steps(run_dir):
            # The run has trained before but nothing intact is left. Starting over would silently
            # throw the run away, so this is an error a person has to look at.
            return finish(EXIT_NO_VALID_CHECKPOINT, "failed", "no_valid_checkpoint", rejected=rejected)
        if latest is not None:
            backend.load_state(latest)
            progress.update(read_json(latest / checkpoint.PROGRESS))
            event(run_dir, "resumed", checkpoint=latest.name, step=progress["step"], skipped=rejected)
        progress["sessions"] += 1
        session_no = progress["sessions"]
        (run_dir / "sessions").mkdir(exist_ok=True)
        from .util import atomic_write_json, now_iso
        atomic_write_json(run_dir / "sessions" / f"session-{session_no:04d}.json",
                          {"session": session_no, "started_at": now_iso(), "start_step": progress["step"],
                           "resumed_from": latest.name if latest else None, "session_hours": session_hours,
                           "stop_at": stop_at, "backend": facts})
        event(run_dir, "session_start", session=session_no, step=progress["step"])
        steps_per_epoch = max(1, backend.steps_per_epoch())
        deadlines = [t for t in (session_started + session_hours * 3600 if session_hours is not None else None, stop_at)
                     if t is not None]
        status.update({"step": progress["step"], "session": session_no, "steps_per_epoch": steps_per_epoch,
                       "resumed_from": latest.name if latest else None,
                       "session_deadline_at": min(deadlines) if deadlines else None})
        write_status(run_dir, status)
        report()
        last_report = clock()

        last_ckpt_step = progress["step"]
        last_ckpt_time = clock()
        session_steps = 0
        session_train_seconds = 0.0

        def save(reason: str) -> None:
            nonlocal last_ckpt_step, last_ckpt_time
            if progress["step"] == last_ckpt_step:
                return  # nothing new since the last checkpoint (or since the start of a fresh run)
            t0 = clock()
            path = checkpoint.save(run_dir, progress["step"], backend.save_state, dict(progress))
            actions = checkpoint.apply_retention(run_dir, ck["keep_resumable"], ck["milestone_every_steps"],
                                                 backend.weight_files)
            event(run_dir, "checkpoint", name=path.name, reason=reason, seconds=round(clock() - t0, 2), retention=actions)
            last_ckpt_step, last_ckpt_time = progress["step"], clock()
            status["latest_checkpoint"] = path.name

        while True:
            step = progress["step"]
            reason: Optional[str] = None
            if step >= limits["max_steps"]:
                reason = "max_steps"
            elif limits["max_epochs"] is not None and step >= limits["max_epochs"] * steps_per_epoch:
                reason = "max_epochs"
            elif limits["max_hours"] is not None and progress["train_seconds"] >= limits["max_hours"] * 3600:
                reason = "max_hours"
            elif (val["early_stopping_patience"] is not None
                  and progress["validations_without_improvement"] >= val["early_stopping_patience"]):
                reason = "early_stopping"
            elif stop_file.exists() or flag.signal:
                reason = "stop_requested"
            elif session_hours is not None and clock() - session_started >= session_hours * 3600:
                reason = "session_time_limit"
            elif stop_at is not None and clock() >= stop_at:
                reason = "stop_at"
            if reason:
                save(reason)
                if reason in RESUMABLE_REASONS:
                    code = EXIT_STOP_REQUESTED if reason == "stop_requested" else EXIT_TIME_LIMIT
                    return finish(code, "stopped", reason, signal=flag.signal)
                return finish(EXIT_COMPLETED, "completed", reason)

            t0 = clock()
            result = backend.train_step(step + 1)
            seconds = clock() - t0
            loss = float(result["loss"])
            if loss != loss or loss in (float("inf"), float("-inf")):
                raise NonFiniteLoss(f"loss = {loss} ở step {step + 1}")
            progress["step"] = step + 1
            progress["train_seconds"] += seconds
            session_steps += 1
            session_train_seconds += seconds
            append_jsonl(run_dir / "metrics.jsonl", {"kind": "step", "step": progress["step"],
                                                     "epoch": round(progress["step"] / steps_per_epoch, 4),
                                                     "seconds": round(seconds, 4), "session": session_no, **result})
            status.update({"step": progress["step"], "epoch": round(progress["step"] / steps_per_epoch, 4),
                           "last_loss": loss, "learning_rate": result.get("lr"),
                           "train_seconds_total": round(progress["train_seconds"], 1),
                           "seconds_per_step": round(session_train_seconds / session_steps, 4),
                           "vram_peak_mib": result.get("vram_peak_mib", status.get("vram_peak_mib"))})
            # An ETA is only shown once this session has measured enough steps.
            status["eta_seconds_to_max_steps"] = (
                round((limits["max_steps"] - progress["step"]) * session_train_seconds / session_steps)
                if session_steps >= MIN_STEPS_FOR_ETA else None)
            write_status(run_dir, status)

            if val["every_steps"] > 0 and progress["step"] % val["every_steps"] == 0:
                v = backend.validate()
                if v is not None:
                    best = progress["best_validation"]
                    improved = best is None or v < best
                    progress["best_validation"] = v if improved else best
                    progress["validations_without_improvement"] = 0 if improved else progress["validations_without_improvement"] + 1
                    append_jsonl(run_dir / "metrics.jsonl", {"kind": "validation", "step": progress["step"],
                                                             "validation_loss": v, "improved": improved})
                    status["validation_loss"] = v
            due_steps = ck["every_steps"] > 0 and progress["step"] - last_ckpt_step >= ck["every_steps"]
            due_time = ck["every_minutes"] > 0 and clock() - last_ckpt_time >= ck["every_minutes"] * 60
            saved = due_steps or due_time
            if saved:
                save("interval")
            status["validations_without_improvement"] = progress["validations_without_improvement"]
            if saved or clock() - last_report >= REPORT_EVERY_SECONDS:
                write_status(run_dir, status)
                report()
                last_report = clock()
    except NonFiniteLoss as e:
        return finish(EXIT_NON_FINITE, "failed", "non_finite_loss", error=str(e))
    except OutOfMemory as e:
        return finish(EXIT_OOM, "failed", "out_of_memory", error=str(e))
    except Mismatch as e:
        return finish(EXIT_MISMATCH, "failed", "mismatch", error=str(e))
    except LushTrainingError as e:
        return finish(EXIT_CRASH, "failed", "error", error=str(e))
    except Exception as e:  # anything else is a crash; the last checkpoint stays as it was
        return finish(EXIT_CRASH, "failed", "crash", error=f"{type(e).__name__}: {e}")
