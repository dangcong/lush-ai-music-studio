"""Style LoRA training (job type `train_lora`): Side-Step as vendored in ACE-Step (`train.py fixed`).

The trainer runs as a separate process and needs the whole GPU, so the inference server is stopped
for the duration of a run and started again afterwards, whatever the outcome (docs/STYLE_TRAINING.md,
D21). Everything reported to the Control Plane comes from the trainer's own output: no value is
estimated or filled in here.
"""
from __future__ import annotations

import hashlib
import json
import logging
import math
import os
import queue
import re
import shutil
import signal
import subprocess
import threading
import time
from dataclasses import dataclass, field
from pathlib import Path
from typing import Any, Callable, Optional

from .audio import probe_wav, to_wav48k
from .engines.base import EngineError, JobContext

log = logging.getLogger(__name__)

JOB_TYPE = "train_lora"
DATASET_KEY_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,63}(/[A-Za-z0-9][A-Za-z0-9._-]{0,63})?$")
LORA_KEY_RE = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]{0,99}$")
ADAPTER_FILES = ("adapter_config.json", "adapter_model.safetensors")
ANSI_RE = re.compile(r"\x1b\[[0-9;]*[A-Za-z]")
# Lines Side-Step prints when stdout is not a TTY (measured at ACE-Step ca1e85f, E1).
STEP_RE = re.compile(r"Epoch (\d+)/(\d+), Step (\d+), Loss: (\S+)")
EPOCH_RE = re.compile(r"\[OK\] Epoch (\d+)/(\d+) in ([\d.]+)s, Loss: (\S+)")
CHECKPOINT_RE = re.compile(r"\[OK\] Checkpoint saved at epoch (\d+)")
COMPLETE_MARK = "[OK] Training complete"

PHASE_STOPPING = "stopping_inference"
PHASE_PREPROCESSING = "preprocessing"
PHASE_TRAINING = "training"
PHASE_SAVING = "saving"
PHASE_RESTORING = "restoring_inference"

METRICS_FLUSH_SECONDS = 5.0
STOP_GRACE_SECONDS = 20.0


class TrainingError(EngineError):
    """A training run failed. Never retryable: a run is restarted only by an Admin."""


MAX_TRAIN_SECONDS = 240


def build_snapshot_dataset(dataset_dir: Path, payload: dict[str, Any],
                           convert: Callable[[Path, Path], None] = to_wav48k,
                           ctx: Optional[JobContext] = None) -> int:
    """Write a Side-Step dataset (`dataset.json` + `audio/sNNN.wav`) from a snapshot payload.
    Built in `<dataset_dir>.partial` and renamed at the end; a failure removes the partial directory.
    An existing dataset is never overwritten. Returns the number of samples written."""
    if dataset_dir.exists():
        raise TrainingError(f"dataset {dataset_dir.name!r} already exists on this node; it is never overwritten")
    partial = dataset_dir.with_name(dataset_dir.name + ".partial")
    shutil.rmtree(partial, ignore_errors=True)
    genre = payload.get("genre") or ""
    inputs = payload.get("inputs") or {}
    try:
        (partial / "audio").mkdir(parents=True)
        entries = []
        for i, sample in enumerate(payload.get("samples") or []):
            if ctx is not None:
                ctx.raise_if_aborted()
            info = inputs.get(sample.get("input"))
            if not info or not info.get("path"):
                raise TrainingError(f"sample {i} has no downloaded input {sample.get('input')!r}")
            name = f"s{i:03d}.wav"
            dest = partial / "audio" / name
            convert(Path(info["path"]), dest)
            duration = int(round(min(probe_wav(dest)[1], MAX_TRAIN_SECONDS)))
            entry: dict[str, Any] = {
                "filename": name, "audio_path": name, "caption": sample.get("train_caption") or "",
                "genre": genre, "lyrics": "[Instrumental]", "is_instrumental": True,
                "language": "unknown", "duration": duration, "labeled": True}
            if sample.get("bpm") is not None:
                entry["bpm"] = sample["bpm"]
            if sample.get("keyscale") is not None:
                entry["keyscale"] = sample["keyscale"]
            entries.append(entry)
        if ctx is not None:
            ctx.raise_if_aborted()
        meta = {"name": str((payload.get("dataset") or {}).get("key") or dataset_dir.name), "custom_tag": "",
                "tag_position": "prepend", "all_instrumental": True, "genre_ratio": 0}
        (partial / "dataset.json").write_text(json.dumps({"metadata": meta, "samples": entries}, indent=2),
                                              encoding="utf-8")
        partial.rename(dataset_dir)
        return len(entries)
    except (RuntimeError, OSError, ValueError) as e:
        shutil.rmtree(partial, ignore_errors=True)
        if isinstance(e, TrainingError):
            raise
        raise TrainingError(f"could not build the dataset: {e}") from e
    except BaseException:
        shutil.rmtree(partial, ignore_errors=True)
        raise


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def gpu_compute_pids() -> Optional[list[int]]:
    """PIDs with a CUDA context, or None when nvidia-smi cannot tell."""
    exe = shutil.which("nvidia-smi")
    if not exe:
        return None
    try:
        proc = subprocess.run([exe, "--query-compute-apps=pid", "--format=csv,noheader"],
                              capture_output=True, text=True, timeout=10)
    except (OSError, subprocess.TimeoutExpired):
        return None
    if proc.returncode != 0:
        return None
    return [int(line) for line in proc.stdout.split() if line.strip().isdigit()]


class InferenceService:
    """The ACE-Step API server as a systemd user unit that this worker may stop and start."""

    def __init__(self, unit: str, ready: Callable[[], bool],
                 run: Callable[..., Any] = subprocess.run,
                 compute_pids: Callable[[], Optional[list[int]]] = gpu_compute_pids,
                 sleep: Callable[[float], None] = time.sleep):
        self.unit = unit
        self._ready = ready
        self._run = run
        self._compute_pids = compute_pids
        self._sleep = sleep

    def _systemctl(self, *args: str) -> Any:
        return self._run(["systemctl", "--user", *args], capture_output=True, text=True, timeout=120)

    def is_active(self) -> bool:
        return self._systemctl("is-active", self.unit).stdout.strip() == "active"

    def stop(self, timeout: float = 60.0) -> None:
        """Stop the server and wait until no process holds a CUDA context any more."""
        proc = self._systemctl("stop", self.unit)
        if proc.returncode != 0:
            raise TrainingError(f"could not stop {self.unit}: {(proc.stderr or proc.stdout).strip()[:300]}")
        deadline = time.monotonic() + timeout
        while True:
            pids = self._compute_pids()
            if not self.is_active() and not pids:
                if pids is None:
                    log.warning("nvidia-smi unavailable: cannot verify that the GPU is free")
                return
            if time.monotonic() >= deadline:
                raise TrainingError(f"GPU still in use after stopping {self.unit} (pids: {pids})")
            self._sleep(1.0)

    def start(self, timeout: float = 300.0) -> bool:
        """Start the server and wait until it has its model loaded. False if it never got there."""
        proc = self._systemctl("start", self.unit)
        if proc.returncode != 0:
            log.error("could not start %s: %s", self.unit, (proc.stderr or proc.stdout).strip()[:300])
            return False
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            try:
                if self._ready():
                    return True
            except Exception as e:  # the server is still coming up
                log.debug("%s not ready yet: %s", self.unit, e)
            self._sleep(2.0)
        return False


@dataclass
class RunState:
    """What the trainer has printed so far."""
    epochs_total: int = 0
    epoch: int = 0
    step: int = 0
    last_loss: Optional[float] = None
    checkpoints: list[int] = field(default_factory=list)
    checkpoint_steps: dict[int, Optional[int]] = field(default_factory=dict)
    completed: bool = False
    resumed_fresh: bool = False
    failure: Optional[str] = None
    oom: bool = False
    tail: list[str] = field(default_factory=list)


class SideStepTrainer:
    job_type = JOB_TYPE

    def __init__(self, acestep_dir: Path, data_dir: Path, lora_dir: Path, service: InferenceService,
                 load_adapter: Optional[Callable[[str, Path], None]] = None,
                 gpu_query: Callable[[], Optional[dict[str, Any]]] = lambda: None,
                 python: Optional[Path] = None, script: Optional[Path] = None):
        self.acestep_dir = acestep_dir
        self.dataset_dir = data_dir / "datasets"
        self.tensor_dir = data_dir / "tensors"
        self.training_dir = data_dir / "training"
        self.lora_dir = lora_dir
        self.service = service
        self._load_adapter = load_adapter
        self._gpu_query = gpu_query
        self.python = python or acestep_dir / ".venv" / "bin" / "python"
        self.script = script or acestep_dir / "train.py"
        # The tensor loader only accepts paths under the working directory it was started in
        # (acestep/training/path_safety.py), so run from the common parent of ACE-Step and the data.
        self.cwd = Path(os.path.commonpath([str(acestep_dir.resolve()), str(data_dir.resolve())]))

    # ---------- inventory ----------
    def list_datasets(self) -> list[dict[str, Any]]:
        """Dataset versions on disk: `<DATA_DIR>/datasets/<name>/<version>/dataset.json` + `audio/`."""
        found: list[dict[str, Any]] = []
        if not self.dataset_dir.is_dir():
            return found
        for manifest in sorted(self.dataset_dir.glob("*/*/dataset.json")):
            key = f"{manifest.parent.parent.name}/{manifest.parent.name}"
            if not DATASET_KEY_RE.match(key):
                continue
            try:
                data = json.loads(manifest.read_text(encoding="utf-8"))
                samples = data.get("samples") or []
                seconds = sum(float(s.get("duration") or 0) for s in samples)
                found.append({"key": key, "name": (data.get("metadata") or {}).get("name") or key,
                              "samples": len(samples), "seconds": round(seconds, 1)})
            except (OSError, ValueError, TypeError, AttributeError) as e:
                log.warning("dataset %s: unreadable dataset.json (%s)", key, e)
        return found

    def recover(self) -> None:
        """After a worker crash mid-run the inference server may still be stopped: bring it back."""
        try:
            if not self.service.is_active():
                log.warning("%s is not running at worker start; starting it", self.service.unit)
                if not self.service.start():
                    log.error("%s did not become ready", self.service.unit)
        except Exception as e:
            log.error("could not check %s: %s", self.service.unit, e)

    # ---------- commands ----------
    def _base_cmd(self, variant: str) -> list[str]:
        return [str(self.python), str(self.script), "--yes", "fixed",
                "--checkpoint-dir", str(self.acestep_dir / "checkpoints"), "--model-variant", variant]

    def preprocess_cmd(self, variant: str, dataset: Path, tensors: Path, out: Path) -> list[str]:
        # `--dataset-dir` / `--output-dir` are required by the parser even though preprocessing ignores them.
        return self._base_cmd(variant) + [
            "--preprocess", "--audio-dir", str(dataset / "audio"), "--dataset-json", str(dataset / "dataset.json"),
            "--tensor-output", str(tensors), "--max-duration", "240",
            "--dataset-dir", str(tensors), "--output-dir", str(out)]

    def train_cmd(self, variant: str, tensors: Path, out: Path, cfg: dict[str, Any]) -> list[str]:
        return self._base_cmd(variant) + [
            "--dataset-dir", str(tensors), "--output-dir", str(out),
            "--rank", str(cfg["rank"]), "--alpha", str(cfg["alpha"]), "--dropout", "0.1",
            "--lr", repr(float(cfg["learning_rate"])), "--batch-size", "1",
            "--gradient-accumulation", str(cfg["gradient_accumulation"]),
            "--epochs", str(cfg["epochs"]), "--warmup-steps", str(cfg["warmup_steps"]),
            "--save-every", str(cfg["save_every"]), "--log-every", "1", "--seed", str(cfg["seed"]),
            "--optimizer-type", "adamw", "--gradient-checkpointing", "--offload-encoder"]

    # ---------- run ----------
    @staticmethod
    def _config(payload: dict[str, Any]) -> dict[str, Any]:
        raw = payload.get("config") or {}
        try:
            cfg = {name: int(raw[name]) for name in
                   ("rank", "alpha", "epochs", "gradient_accumulation", "save_every", "warmup_steps", "seed")}
            cfg["learning_rate"] = float(raw["learning_rate"])
        except (KeyError, TypeError, ValueError) as e:
            raise TrainingError(f"invalid training config: {e!r}") from e
        if min(cfg["rank"], cfg["alpha"], cfg["epochs"], cfg["gradient_accumulation"], cfg["save_every"]) < 1 \
                or cfg["warmup_steps"] < 0 or not 0 < cfg["learning_rate"] <= 0.01:
            raise TrainingError(f"training config out of range: {cfg}")
        return cfg

    def run(self, ctx: JobContext, payload: dict[str, Any]) -> dict[str, Any]:
        cfg = self._config(payload)
        self._sequence = 0
        lora_key = str((payload.get("lora") or {}).get("key") or "")
        dataset_key = str((payload.get("dataset") or {}).get("key") or "")
        model_key = str((payload.get("base_model") or {}).get("key") or "")
        if not LORA_KEY_RE.match(lora_key):
            raise TrainingError(f"invalid LoRA key {lora_key!r}")
        if not DATASET_KEY_RE.match(dataset_key):
            raise TrainingError(f"invalid dataset key {dataset_key!r}")
        if not model_key.startswith("acestep-v15-"):
            raise TrainingError(f"no trainer for base model {model_key!r}")
        variant = model_key[len("acestep-v15-"):].replace("-", "_")
        dataset = self.dataset_dir / dataset_key
        snapshot = bool(payload.get("samples"))
        if snapshot and not LORA_KEY_RE.match(dataset_key):
            raise TrainingError(f"invalid dataset key {dataset_key!r}")
        # Cheap preconditions first: a failure here must not leave a converted dataset on disk.
        if not snapshot and (not (dataset / "dataset.json").is_file() or not (dataset / "audio").is_dir()):
            raise TrainingError(f"dataset {dataset_key!r} is not on this node")
        if not (self.acestep_dir / "checkpoints" / model_key).is_dir():
            raise TrainingError(f"checkpoint {model_key!r} is not on this node")
        target = self.lora_dir / lora_key
        if target.exists():
            raise TrainingError(f"LoRA {lora_key!r} already exists on this node; it is never overwritten")
        if snapshot:
            n = len(payload["samples"])
            self._report(ctx, 0.01, f"preparing {n} samples", "preparing_dataset")
            build_snapshot_dataset(dataset, payload, convert=to_wav48k, ctx=ctx)
        samples = len(json.loads((dataset / "dataset.json").read_text(encoding="utf-8")).get("samples") or [])
        if samples == 0:
            raise TrainingError(f"dataset {dataset_key!r} has no samples")

        run_dir = self.training_dir / ctx.job_id
        out = run_dir / "out"
        run_dir.mkdir(parents=True, exist_ok=True)
        tensors = self.tensor_dir / f"{dataset_key.replace('/', '-')}-{model_key}"
        max_seconds = float((payload.get("limits") or {}).get("max_seconds") or 0)
        started = time.monotonic()
        deadline = started + max_seconds if max_seconds > 0 else None
        result: dict[str, Any] = {"trainer": "sidestep", "acestep_commit": self._acestep_commit(),
                                  "run_dir": str(run_dir), "samples": samples}
        state = RunState(epochs_total=cfg["epochs"])
        stop_reason = "completed"
        vram_peak = 0

        self._report(ctx, 0.02, f"stopping {self.service.unit}: music generation is paused", PHASE_STOPPING)
        ctx.raise_if_aborted()
        self.service.stop()
        try:
            # ---- preprocess (cached per dataset version + base model) ----
            if len(list(tensors.glob("*.pt"))) < samples:
                self._report(ctx, 0.05, f"preprocessing {samples} samples", PHASE_PREPROCESSING)
                t0 = time.monotonic()
                pre = RunState()
                code, _ = self._spawn(ctx, self.preprocess_cmd(variant, dataset, tensors, out),
                                      run_dir / "preprocess.log", pre, deadline, phase=PHASE_PREPROCESSING)
                result["preprocess_seconds"] = round(time.monotonic() - t0, 1)
                done = len(list(tensors.glob("*.pt")))
                if code != 0 or done < samples:
                    raise TrainingError(self._why(pre, code, f"preprocessing produced {done}/{samples} tensors"))
            else:
                self._report(ctx, 0.05, f"tensors already preprocessed ({samples} samples)", PHASE_PREPROCESSING)

            # ---- train ----
            steps_total = math.ceil(samples / cfg["gradient_accumulation"]) * cfg["epochs"]
            self._report(ctx, 0.1, f"training: {cfg['epochs']} epochs, {steps_total} optimizer steps",
                         PHASE_TRAINING)
            t0 = time.monotonic()
            code, vram_peak = self._spawn(ctx, self.train_cmd(variant, tensors, out, cfg), run_dir / "train.log",
                                          state, deadline, phase=PHASE_TRAINING, steps_total=steps_total)
            result["train_seconds"] = round(time.monotonic() - t0, 1)
            if state.failure == "time_limit":
                stop_reason = "time_limit"
            elif code != 0 or not state.completed:
                raise TrainingError(self._why(state, code, "the trainer stopped without finishing"))

            # ---- pick the adapter ----
            self._report(ctx, 0.92, "checking the adapter", PHASE_SAVING)
            checkpoints = self._checkpoints(out, state.checkpoints, state.checkpoint_steps)
            if stop_reason == "time_limit":
                if not checkpoints:
                    raise TrainingError("time limit reached before the first checkpoint was saved")
                adapter_dir = Path(checkpoints[-1]["path"])
            else:
                adapter_dir = out / "final"
            for name in ADAPTER_FILES:
                if not (adapter_dir / name).is_file() or (adapter_dir / name).stat().st_size == 0:
                    raise TrainingError(f"adapter file missing or empty: {adapter_dir / name}")
            adapter = adapter_dir / "adapter_model.safetensors"
            result["lora"] = {"key": lora_key, "adapter_sha256": sha256_file(adapter),
                              "bytes": adapter.stat().st_size, "source": adapter_dir.name}
            reported = list(checkpoints)
            final_epoch = state.epoch or state.epochs_total
            if stop_reason == "completed" and final_epoch >= 1 and all(c["epoch"] != final_epoch for c in reported):
                reported.append({"epoch": final_epoch, "step": state.step or None, "path": str(adapter_dir.resolve()),
                                 "sha256": result["lora"]["adapter_sha256"], "bytes": self._dir_bytes(adapter_dir)})
            result["checkpoints"] = reported
            self._publish(adapter_dir, target)
        finally:
            ctx_message = f"starting {self.service.unit}"
            self._report(ctx, 0.95, ctx_message, PHASE_RESTORING)
            restored = self.service.start()
            result["inference_restored"] = restored
            if not restored:
                log.error("%s did not come back after training; this node cannot generate music", self.service.unit)

        result.update({"stop_reason": stop_reason, "epochs_completed": state.epoch,
                       "optimizer_steps": state.step, "vram_peak_mb": vram_peak or None,
                       "total_seconds": round(time.monotonic() - started, 1)})
        # A LoRA that inference cannot load must not reach the registry.
        if restored and self._load_adapter is not None:
            try:
                self._load_adapter(lora_key, target)
                result["adapter_loadable"] = True
            except Exception as e:
                shutil.rmtree(target, ignore_errors=True)
                raise TrainingError(f"the trained adapter could not be loaded by inference: {e}") from e
        (run_dir / "run.json").write_text(json.dumps({"job_id": ctx.job_id, "payload": payload, "result": result},
                                                     indent=2), encoding="utf-8")
        return result

    # ---------- helpers ----------
    @staticmethod
    def _report(ctx: JobContext, value: float, message: Optional[str], phase: str,
                metrics: Optional[list[dict[str, Any]]] = None) -> None:
        ctx.report(value, message, phase=phase, metrics=metrics)  # type: ignore[attr-defined]

    def _acestep_commit(self) -> Optional[str]:
        try:
            proc = subprocess.run(["git", "-C", str(self.acestep_dir), "rev-parse", "HEAD"],
                                  capture_output=True, text=True, timeout=10)
            return proc.stdout.strip() or None if proc.returncode == 0 else None
        except (OSError, subprocess.TimeoutExpired):
            return None

    @staticmethod
    def _why(state: RunState, code: Optional[int], fallback: str) -> str:
        if state.oom:
            return "out of GPU memory (CUDA out of memory)"
        if state.failure and state.failure != "time_limit":
            return state.failure
        tail = " | ".join(state.tail[-3:])
        return f"{fallback} (exit code {code}){': ' + tail if tail else ''}"

    @staticmethod
    def _dir_bytes(path: Path) -> int:
        return sum(f.stat().st_size for f in path.iterdir() if f.is_file())

    @staticmethod
    def _checkpoints(out: Path, reported: list[int],
                     steps: Optional[dict[int, Optional[int]]] = None) -> list[dict[str, Any]]:
        """Checkpoints the trainer announced AND that are complete on disk, oldest first. Each epoch
        appears once (the first complete directory for it)."""
        found = []
        for epoch in sorted(set(reported)):
            for path in sorted((out / "checkpoints").glob(f"epoch_{epoch}_loss_*")):
                files = [path / name for name in ADAPTER_FILES]
                if all(f.is_file() and f.stat().st_size > 0 for f in files):
                    found.append({"epoch": epoch, "step": (steps or {}).get(epoch),
                                  "path": str(path.resolve()),
                                  "sha256": sha256_file(path / "adapter_model.safetensors"),
                                  "bytes": SideStepTrainer._dir_bytes(path)})
                    break
        return found

    @staticmethod
    def _publish(adapter_dir: Path, target: Path) -> None:
        """Copy the adapter into LORA_DIR under its key. Never overwrites an existing LoRA."""
        staging = target.with_name(target.name + ".partial")
        shutil.rmtree(staging, ignore_errors=True)
        staging.mkdir(parents=True)
        for name in ADAPTER_FILES:
            shutil.copy2(adapter_dir / name, staging / name)
        staging.rename(target)

    @staticmethod
    def _terminate(proc: subprocess.Popen) -> None:
        """Side-Step has no signal handler: stopping loses everything after the last checkpoint."""
        if proc.poll() is not None:
            return
        try:
            if os.name == "posix":
                os.killpg(proc.pid, signal.SIGINT)
            else:
                proc.terminate()
            proc.wait(timeout=STOP_GRACE_SECONDS)
        except (OSError, subprocess.TimeoutExpired):
            try:
                if os.name == "posix":
                    os.killpg(proc.pid, signal.SIGKILL)
                else:
                    proc.kill()
            except OSError:
                pass
            proc.wait(timeout=30)

    def _parse(self, line: str, state: RunState, started: float, sequence: int) -> Optional[dict[str, Any]]:
        """Update `state` from one output line; return a metric event for a step line."""
        low = line.lower()
        if "out of memory" in low:
            state.oom = True
        if "starting fresh" in low:
            state.resumed_fresh = True
        if "[FAIL]" in line and state.failure is None:
            state.failure = line.split("[FAIL]", 1)[1].strip()[:300] or "trainer reported a failure"
        if COMPLETE_MARK in line:
            state.completed = True
        if m := CHECKPOINT_RE.search(line):
            state.checkpoints.append(int(m.group(1)))
            state.checkpoint_steps[int(m.group(1))] = state.step or None
        if m := EPOCH_RE.search(line):
            state.epoch = int(m.group(1))
        if m := STEP_RE.search(line):
            try:
                loss = float(m.group(4))
            except ValueError:
                loss = float("nan")
            state.step = int(m.group(3))
            if not math.isfinite(loss):
                state.failure = f"loss is not finite at optimizer step {state.step} ({m.group(4)})"
                return None
            state.last_loss = loss
            gpu = self._gpu_query()
            return {"sequence": sequence, "global_step": state.step, "epoch": int(m.group(1)),
                    "train_loss": loss, "learning_rate": None,
                    "elapsed_seconds": round(time.monotonic() - started, 1),
                    "gpu_memory_mb": gpu.get("vram_used_mb") if gpu else None}
        return None

    def _spawn(self, ctx: JobContext, cmd: list[str], log_path: Path, state: RunState,
               deadline: Optional[float], phase: str, steps_total: int = 0) -> tuple[Optional[int], int]:
        """Run one trainer command, streaming its output. Returns (exit code, peak VRAM in MB)."""
        env = {**os.environ, "PYTHONUNBUFFERED": "1"}
        proc = subprocess.Popen(cmd, cwd=str(self.cwd), env=env, stdin=subprocess.DEVNULL,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True,
                                encoding="utf-8", errors="replace", bufsize=1,
                                start_new_session=(os.name == "posix"))
        lines: "queue.Queue[Optional[str]]" = queue.Queue()

        def pump() -> None:
            assert proc.stdout is not None
            for raw in proc.stdout:
                lines.put(raw)
            lines.put(None)

        threading.Thread(target=pump, name="trainer-output", daemon=True).start()
        started = time.monotonic()
        pending: list[dict[str, Any]] = []
        last_flush = started
        sequence = int(getattr(self, "_sequence", 0))
        vram_peak = 0
        try:
            with open(log_path, "a", encoding="utf-8") as logfile:
                while True:
                    try:
                        raw = lines.get(timeout=1.0)
                    except queue.Empty:
                        raw = ""
                    if raw is None:
                        break
                    line = ANSI_RE.sub("", raw).strip()
                    message = None
                    if line:
                        logfile.write(line + "\n")
                        logfile.flush()
                        state.tail = (state.tail + [line[:300]])[-5:]
                        event = self._parse(line, state, started, sequence + 1)
                        if event:
                            sequence += 1
                            pending.append(event)
                            vram_peak = max(vram_peak, event["gpu_memory_mb"] or 0)
                        if EPOCH_RE.search(line) or CHECKPOINT_RE.search(line) or "[FAIL]" in line \
                                or "Resumed from" in line or COMPLETE_MARK in line:
                            message = line[:300]
                    now = time.monotonic()
                    if message or (pending and now - last_flush >= METRICS_FLUSH_SECONDS):
                        value = 0.05 if not steps_total else 0.1 + 0.8 * min(1.0, state.step / steps_total)
                        self._report(ctx, value, message, phase, pending or None)
                        pending, last_flush = [], now
                    # Reasons to stop the trainer early.
                    if state.failure and state.failure != "time_limit" or state.oom:
                        self._terminate(proc)
                        break
                    if deadline is not None and now >= deadline:
                        state.failure = "time_limit"
                        self._terminate(proc)
                        break
                    if ctx.cancel_requested or ctx.lease_lost or ctx.shutdown_requested:
                        self._terminate(proc)
                        ctx.raise_if_aborted()
        except BaseException:
            self._terminate(proc)
            raise
        finally:
            self._sequence = sequence
        if pending:
            self._report(ctx, 0.1 + 0.8 * min(1.0, state.step / steps_total) if steps_total else 0.05,
                         None, phase, pending)
        return proc.wait(), vram_peak


def build_trainer(config: Any, engines: dict[str, Any],
                  gpu_query: Callable[[], Optional[dict[str, Any]]] = lambda: None) -> Optional[SideStepTrainer]:
    """The trainer for this node, or None when training is not configured (the node then does not
    advertise the `train_lora` job type)."""
    engine = engines.get("acestep")
    if engine is None or not (config.acestep_dir and config.data_dir and config.lora_dir):
        return None
    from .engines.acestep import adapter_name

    def ready() -> bool:
        return bool(engine.client.health().get("models_initialized"))

    def load_adapter(key: str, path: Path) -> None:
        engine.client.lora_load(str(path.resolve()), adapter_name=adapter_name(key))
        engine.client.lora_toggle(False)  # loaded for the check only; each job sets its own state

    return SideStepTrainer(config.acestep_dir, config.data_dir, config.lora_dir,
                           InferenceService(config.acestep_service, ready), load_adapter, gpu_query)
