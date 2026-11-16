"""ACE-Step 1.5 DiT fine-tuning backend (`acestep15-dit`).

STATUS: written against the source of ACE-Step commit ca1e85f, NOT YET RUN ON A GPU. Nothing in
this file has been executed with a real checkpoint. See docs/STATUS.md before relying on it.

Upstream has no full fine-tuning trainer at this commit (`load_decoder_for_training` freezes every
parameter; its guide says full-parameter SFT is "NOT natively supported"). This backend therefore:

- imports, unchanged, from the pinned checkout: model loading, the tensor dataset and collate
  function, continuous timestep sampling, CFG dropout, gradient-checkpointing setup;
- reproduces the training step of `FixedLoRAModule.training_step` (flow matching: x_t = t*x1 +
  (1-t)*x0, target x1 - x0, MSE) without injecting an adapter;
- owns what upstream lacks: unfreezing the decoder, the optimizer step, validation loss, full
  state save / load (weights, optimizer, scheduler, RNG, data order).

Must run with the Python of the ACE-Step virtualenv (torch, transformers, safetensors live there).
"""
from __future__ import annotations

import math
import random
import re
import sys
from pathlib import Path
from typing import Any, Optional

from . import model as model_info
from .acestep_probe import probe_repo
from .config import Project
from .run import load_run, parent_sha_matches, tensor_dir
from .session import Mismatch, OutOfMemory
from .util import LushTrainingError, read_json

DECODER_FILE = "decoder.safetensors"
BATCH_KEYS = ("target_latents", "attention_mask", "encoder_hidden_states", "encoder_attention_mask", "context_latents")


class AceStepDecoderBackend:
    weight_files = (DECODER_FILE,)

    def __init__(self, project: Project, run_dir: Path, device: str = "cuda"):
        self.project = project
        self.run_dir = Path(run_dir)
        self.info = load_run(run_dir)
        self.cfg = self.info["config"]
        self.device = device
        self.order: list[int] = []
        self.position = 0
        self.epoch_index = 0

    # ---------- setup ----------
    def setup(self) -> dict[str, Any]:
        repo = self.project.acestep_repo
        if repo is None:
            raise LushTrainingError("chưa khai báo acestep.repo_dir trong configs/project.yaml")
        probe = probe_repo(repo, self.project.acestep_commit)
        if not probe["ok"]:
            raise Mismatch(f"ACE-Step checkout không khớp bản đã ghim: {probe['problems']}")
        if probe["commit"] != self.info["code"]["acestep_commit"]:
            raise Mismatch("commit ACE-Step đã đổi so với lúc tạo run")
        if not parent_sha_matches(self.project, self.info):
            raise Mismatch("SHA-256 của checkpoint gốc không khớp run.json")
        sys.path.insert(0, str(repo))
        import numpy as np
        import torch
        from acestep.training.data_module import PreprocessedTensorDataset, collate_preprocessed_batch
        from acestep.training.path_safety import set_safe_root
        from acestep.training_v2.model_loader import load_decoder_for_training
        from acestep.training_v2.timestep_sampling import apply_cfg_dropout, sample_timesteps
        from acestep.training_v2.trainer_helpers import configure_memory_features

        self.torch, self.np = torch, np
        self._apply_cfg_dropout, self._sample_timesteps, self._collate = apply_cfg_dropout, sample_timesteps, collate_preprocessed_batch
        cfg = self.cfg
        set_safe_root(str(self.project.data_root))  # the tensor dataset only reads below its safe root

        seed = int(cfg["seed"])
        random.seed(seed)
        np.random.seed(seed)
        torch.manual_seed(seed)
        self.order_rng = np.random.default_rng(seed)

        self.train_set = PreprocessedTensorDataset(str(tensor_dir(self.project, cfg, "train")))
        expected = self.info["dataset"]["segments"]
        if len(self.train_set) != expected["train"]:
            raise Mismatch(f"có {len(self.train_set)} tensor train, dataset khóa {expected['train']} segment. Chạy lại preprocess.")
        self.val_set = None
        if cfg["validation"]["every_steps"] > 0:
            if expected["validation"] == 0:
                raise LushTrainingError("validation.every_steps > 0 nhưng dataset không có tập validation")
            self.val_set = PreprocessedTensorDataset(str(tensor_dir(self.project, cfg, "validation")))
            if len(self.val_set) != expected["validation"]:
                raise Mismatch(f"có {len(self.val_set)} tensor validation, dataset khóa {expected['validation']} segment")

        # fp32 master weights; the forward pass runs under autocast (cfg.precision.autocast).
        self.model = load_decoder_for_training(str(self.project.checkpoints_dir), self.info["parent"]["name"],
                                               self.device, "fp32")
        patterns = [re.compile(p) for p in cfg["trainable_patterns"]]
        trainable = 0
        for name, p in self.model.named_parameters():
            in_decoder = name.startswith("decoder.")
            p.requires_grad = in_decoder and (cfg["method"] == "full_decoder" or any(r.search(name) for r in patterns))
            trainable += p.numel() if p.requires_grad else 0
        self.params = [p for p in self.model.parameters() if p.requires_grad]
        planned = self.info["training"]["trainable_parameters"]
        if trainable != planned:
            raise Mismatch(f"{trainable:,} tham số requires_grad, run.json ghi {planned:,}. Không train với số khác kế hoạch.")
        self.model.decoder.train()
        ckpt_on = False
        if cfg["gradient_checkpointing"]:
            ckpt_on = bool(configure_memory_features(self.model.decoder)[0])
        self.force_input_grads = ckpt_on
        self.autocast_dtype = torch.bfloat16 if cfg["precision"]["autocast"] == "bf16" else None
        mcfg = self.info["parent"]["config"] or {}
        self.timestep = {"timestep_mu": mcfg.get("timestep_mu", -0.4) if mcfg.get("timestep_mu") is not None else -0.4,
                         "timestep_sigma": mcfg.get("timestep_sigma", 1.0) if mcfg.get("timestep_sigma") is not None else 1.0,
                         "data_proportion": mcfg.get("data_proportion", 0.5) if mcfg.get("data_proportion") is not None else 0.5}
        self.null_cond = getattr(self.model, "null_condition_emb", None)

        self.optimizer = self._build_optimizer()
        self.scheduler = torch.optim.lr_scheduler.LambdaLR(self.optimizer, self._lr_factor)
        self._new_epoch()
        return {
            "trainable_parameters": trainable,
            "total_parameters_loaded": sum(p.numel() for p in self.model.parameters()),
            "trainable_by_component": self._count_by_component(True),
            "frozen_by_component": self._count_by_component(False),
            "gradient_checkpointing_enabled": ckpt_on,
            "timestep": self.timestep, "cfg_dropout": cfg["cfg_dropout"] if self.null_cond is not None else 0.0,
            "dependencies": self._dependencies(),
            "device": torch.cuda.get_device_name(0) if torch.cuda.is_available() else self.device,
        }

    def _count_by_component(self, trainable: bool) -> dict[str, int]:
        out: dict[str, int] = {}
        for name, p in self.model.named_parameters():
            if p.requires_grad == trainable:
                key = model_info.component_of(name)
                out[key] = out.get(key, 0) + p.numel()
        return out

    def _dependencies(self) -> dict[str, Optional[str]]:
        out: dict[str, Optional[str]] = {"python": sys.version.split()[0]}
        for name in ("torch", "transformers", "diffusers", "safetensors", "numpy", "bitsandbytes"):
            try:
                out[name] = str(__import__(name).__version__)
            except Exception:
                out[name] = None
        return out

    def _build_optimizer(self) -> Any:
        opt = self.cfg["optimizer"]
        kind, lr, wd = opt["type"], float(opt["learning_rate"]), float(opt["weight_decay"])
        if kind == "adamw":
            return self.torch.optim.AdamW(self.params, lr=lr, weight_decay=wd)
        if kind == "adamw8bit":
            try:
                import bitsandbytes as bnb
            except ImportError as e:
                raise LushTrainingError("optimizer adamw8bit cần bitsandbytes; chưa cài. Không tự đổi optimizer.") from e
            return bnb.optim.AdamW8bit(self.params, lr=lr, weight_decay=wd)
        from transformers.optimization import Adafactor
        return Adafactor(self.params, lr=lr, weight_decay=wd, relative_step=False, scale_parameter=False)

    def _lr_factor(self, step: int) -> float:
        """Defined over the fixed limits.max_steps of run.json, so a resume cannot bend the schedule."""
        sch, total = self.cfg["scheduler"], self.cfg["limits"]["max_steps"]
        warmup = sch["warmup_steps"]
        if step < warmup:
            return (step + 1) / warmup
        if sch["type"] == "constant":
            return 1.0
        done = min(1.0, (step - warmup) / max(1, total - warmup))
        floor = float(sch["min_lr_ratio"])
        return floor + (1.0 - floor) * 0.5 * (1.0 + math.cos(math.pi * done))

    # ---------- data order ----------
    def steps_per_epoch(self) -> int:
        per_step = self.cfg["batch_size"] * self.cfg["gradient_accumulation"]
        return max(1, math.ceil(len(self.train_set) / per_step))

    def _new_epoch(self) -> None:
        self.order = [int(i) for i in self.order_rng.permutation(len(self.train_set))]
        self.position = 0

    def _next_batch(self) -> dict[str, Any]:
        """Every train segment is seen once per pass, in an order drawn from the seeded generator."""
        items = []
        for _ in range(self.cfg["batch_size"]):
            if self.position >= len(self.order):
                self.epoch_index += 1
                self._new_epoch()
            items.append(self.train_set[self.order[self.position]])
            self.position += 1
        return self._collate(items)

    # ---------- training ----------
    def _loss(self, batch: dict[str, Any], *, train: bool) -> Any:
        torch = self.torch
        dev = self.device
        t = {k: batch[k].to(dev, dtype=torch.float32) for k in BATCH_KEYS}
        ctx = torch.autocast(device_type="cuda", dtype=self.autocast_dtype) if (
            self.autocast_dtype is not None and dev.startswith("cuda")) else _Null()
        with ctx:
            ehs = t["encoder_hidden_states"]
            if train and self.null_cond is not None and self.cfg["cfg_dropout"] > 0:
                ehs = self._apply_cfg_dropout(ehs, self.null_cond, cfg_ratio=float(self.cfg["cfg_dropout"]))
            x0 = t["target_latents"]
            x1 = torch.randn_like(x0)
            ts, _ = self._sample_timesteps(batch_size=x0.shape[0], device=dev, dtype=torch.float32,
                                           use_meanflow=False, **self.timestep)
            t_ = ts.unsqueeze(-1).unsqueeze(-1)
            xt = t_ * x1 + (1.0 - t_) * x0
            if train and self.force_input_grads:
                xt = xt.requires_grad_(True)
            out = self.model.decoder(hidden_states=xt, timestep=ts, timestep_r=ts, attention_mask=t["attention_mask"],
                                     encoder_hidden_states=ehs, encoder_attention_mask=t["encoder_attention_mask"],
                                     context_latents=t["context_latents"])
            loss = torch.nn.functional.mse_loss(out[0].float(), (x1 - x0).float())
        return loss.float()

    def train_step(self, step: int) -> dict[str, Any]:
        torch = self.torch
        accum = self.cfg["gradient_accumulation"]
        try:
            self.optimizer.zero_grad(set_to_none=True)
            total = 0.0
            for _ in range(accum):
                loss = self._loss(self._next_batch(), train=True)
                value = float(loss.detach())
                if not math.isfinite(value):
                    return {"loss": value, "lr": self.scheduler.get_last_lr()[0]}  # session stops; no optimizer step
                (loss / accum).backward()
                total += value
            grad_norm = float(torch.nn.utils.clip_grad_norm_(self.params, float(self.cfg["max_grad_norm"])
                                                              if self.cfg["max_grad_norm"] else float("inf")))
            if not math.isfinite(grad_norm):
                return {"loss": float("nan"), "lr": self.scheduler.get_last_lr()[0], "grad_norm": grad_norm}
            lr = self.scheduler.get_last_lr()[0]
            self.optimizer.step()
            self.scheduler.step()
        except torch.cuda.OutOfMemoryError as e:
            raise OutOfMemory(str(e).splitlines()[0]) from e
        result = {"loss": total / accum, "lr": lr, "grad_norm": grad_norm, "micro_batches": accum,
                  "pass": self.epoch_index + 1}
        if torch.cuda.is_available():
            result["vram_peak_mib"] = int(torch.cuda.max_memory_allocated() // 2**20)
            result["vram_reserved_mib"] = int(torch.cuda.max_memory_reserved() // 2**20)
        return result

    def validate(self) -> Optional[float]:
        """Mean loss over the fixed validation set with fixed noise and timesteps (seeded), no CFG dropout.

        The training RNG stream is saved and restored around it, so validating does not change training.
        """
        if self.val_set is None:
            return None
        torch = self.torch
        cpu_state = torch.get_rng_state()
        cuda_state = torch.cuda.get_rng_state_all() if torch.cuda.is_available() else None
        self.model.decoder.eval()
        try:
            torch.manual_seed(int(self.cfg["validation"]["seed"]))
            losses = []
            with torch.no_grad():
                for i in range(len(self.val_set)):
                    losses.append(float(self._loss(self._collate([self.val_set[i]]), train=False)))
        except torch.cuda.OutOfMemoryError as e:
            raise OutOfMemory(str(e).splitlines()[0]) from e
        finally:
            self.model.decoder.train()
            torch.set_rng_state(cpu_state)
            if cuda_state is not None:
                torch.cuda.set_rng_state_all(cuda_state)
        return sum(losses) / len(losses)

    # ---------- state ----------
    def save_state(self, directory: Path) -> None:
        torch = self.torch
        from safetensors.torch import save_file
        # Full names (`decoder.<...>`) so the export can match them against the parent file directly.
        weights = {f"decoder.{k}": v.detach().to("cpu", dtype=torch.float32).contiguous()
                   for k, v in self.model.decoder.state_dict().items() if v.is_floating_point()}
        save_file(weights, str(directory / DECODER_FILE), metadata={"format": "pt", "run_id": self.info["run_id"]})
        torch.save(self.optimizer.state_dict(), directory / "optimizer.pt")
        torch.save(self.scheduler.state_dict(), directory / "scheduler.pt")
        torch.save({"python": random.getstate(), "numpy": self.np.random.get_state(), "torch_cpu": torch.get_rng_state(),
                    "torch_cuda": torch.cuda.get_rng_state_all() if torch.cuda.is_available() else None,
                    "order_rng": self.order_rng.bit_generator.state, "order": self.order, "position": self.position,
                    "epoch_index": self.epoch_index}, directory / "rng.pt")

    def load_state(self, directory: Path) -> None:
        """Only called on a checkpoint whose manifest and SHA-256 were just verified (the .pt files are pickles)."""
        torch = self.torch
        from safetensors.torch import load_file
        weights = {k[len("decoder."):]: v for k, v in load_file(str(directory / DECODER_FILE)).items()}
        missing, unexpected = self.model.decoder.load_state_dict(weights, strict=False)
        float_missing = [k for k in missing if self.model.decoder.state_dict()[k].is_floating_point()]
        if float_missing or unexpected:
            raise Mismatch(f"checkpoint không khớp decoder: thiếu {float_missing[:5]}, thừa {list(unexpected)[:5]}")
        self.optimizer.load_state_dict(torch.load(directory / "optimizer.pt", map_location=self.device, weights_only=False))
        self.scheduler.load_state_dict(torch.load(directory / "scheduler.pt", weights_only=False))
        rng = torch.load(directory / "rng.pt", weights_only=False)
        random.setstate(rng["python"])
        self.np.random.set_state(rng["numpy"])
        torch.set_rng_state(rng["torch_cpu"])
        if rng["torch_cuda"] is not None and torch.cuda.is_available():
            torch.cuda.set_rng_state_all(rng["torch_cuda"])
        self.order_rng.bit_generator.state = rng["order_rng"]
        self.order, self.position, self.epoch_index = rng["order"], rng["position"], rng["epoch_index"]


class _Null:
    def __enter__(self) -> None:
        return None

    def __exit__(self, *exc: Any) -> bool:
        return False


# ---------- preprocessing (upstream CLI, separate process) ----------

def preprocess_command(project: Project, cfg: dict[str, Any], split: str, engine_json: Path, out: Path) -> list[str]:
    """`train.py fixed --preprocess` as measured on the render node (E1, 2026-11-13, variant turbo, Linux).

    `--dataset-dir` / `--output-dir` are required by the parser even though preprocessing ignores them.
    """
    repo = project.acestep_repo
    python = project.acestep_python
    if repo is None or python is None:
        raise LushTrainingError("cần acestep.repo_dir và acestep.python trong configs/project.yaml")
    return [str(python), str(repo / "train.py"), "--yes", "fixed", "--checkpoint-dir", str(project.checkpoints_dir),
            "--model-variant", cfg["model"]["checkpoint"], "--preprocess", "--dataset-json", str(engine_json),
            "--audio-dir", str(engine_json.parent), "--tensor-output", str(out), "--max-duration", "240",
            "--dataset-dir", str(out), "--output-dir", str(out / "_unused")]


def write_engine_json(vdir: Path, caption_version: str, split: str, out: Path) -> Path:
    """Dataset JSON for the engine with absolute audio paths (the locked copy stays machine-independent)."""
    from .util import atomic_write_json
    doc = read_json(Path(vdir) / "captions" / caption_version / f"engine.{split}.json")
    for s in doc["samples"]:
        s["audio_path"] = str((Path(vdir) / "segments" / split / s["filename"]).resolve())
    path = out / "engine.json"
    atomic_write_json(path, doc)
    return path
