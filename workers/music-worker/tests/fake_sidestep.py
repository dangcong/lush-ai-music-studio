"""Stand-in for ACE-Step's `train.py fixed` in unit tests. Prints the same lines Side-Step printed in
the E1 run on the real GPU and writes files with the same layout. FAKE_MODE selects a failure."""
import json
import math
import os
import sys
import time
from pathlib import Path

args = sys.argv[1:]
mode = os.environ.get("FAKE_MODE", "ok")


def opt(name, default=None):
    return args[args.index(name) + 1] if name in args else default


def save(path: Path, state: bool) -> None:
    path.mkdir(parents=True, exist_ok=True)
    (path / "adapter_config.json").write_text("{}")
    (path / "adapter_model.safetensors").write_bytes(b"weights:" + path.name.encode())
    if state:
        (path / "training_state.pt").write_bytes(b"state")


if "--preprocess" in args:
    samples = json.loads(Path(opt("--dataset-json")).read_text())["samples"]
    out = Path(opt("--tensor-output"))
    out.mkdir(parents=True, exist_ok=True)
    if mode == "preprocess_fail":
        print("[FAIL] Preprocessing failed: boom", flush=True)
        sys.exit(1)
    for sample in samples:
        (out / (Path(sample["filename"]).stem + ".pt")).write_bytes(b"tensor")
    print(f"[OK] Preprocessing complete:\n     Processed: {len(samples)}/{len(samples)}", flush=True)
    sys.exit(0)

out = Path(opt("--output-dir"))
epochs, save_every = int(opt("--epochs")), int(opt("--save-every"))
accumulation = int(opt("--gradient-accumulation"))
samples = len(list(Path(opt("--dataset-dir")).glob("*.pt")))
steps_per_epoch = math.ceil(samples / accumulation)
print(f"[OK] Loaded {samples} preprocessed samples", flush=True)
if mode == "oom":
    print("torch.OutOfMemoryError: CUDA out of memory. Tried to allocate 2.00 GiB", flush=True)
    sys.exit(1)
step = 0
for epoch in range(1, 1 if mode == "silent" else epochs + 1):  # silent: no epoch lines at all
    for _ in range(steps_per_epoch):
        step += 1
        loss = "nan" if mode == "nan" and step == 2 else f"{0.9 - step * 0.01:.4f}"
        print(f"\x1b[1mEpoch {epoch}/{epochs}, Step {step}, Loss: {loss}\x1b[0m", flush=True)
        if mode == "slow":
            time.sleep(0.4)
    print(f"[OK] Epoch {epoch}/{epochs} in 0.1s, Loss: 0.8000", flush=True)
    if epoch % save_every == 0:
        save(out / "checkpoints" / f"epoch_{epoch}_loss_0.8000", state=True)
        print(f"[OK] Checkpoint saved at epoch {epoch}", flush=True)
    if mode == "crash" and epoch == 1:
        print("Traceback (most recent call last):\nRuntimeError: something broke", flush=True)
        sys.exit(1)
save(out / "final", state=False)
print(f"[OK] Training complete! LoRA saved to {out / 'final'}", flush=True)
