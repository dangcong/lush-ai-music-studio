#!/usr/bin/env bash
# Install the Dataset Factory (this workspace) on the rtx6000 render node. Idempotent.
# Run ON rtx6000 from the synced checkout:  bash scripts/install_rtx6000.sh [--captioner]
#   --captioner  also install transformers/accelerate + CUDA torch (cu128) for acestep-captioner.
# Needs user approval before running (installs packages). Does not touch ACE-Step-1.5/ or lush-worker/.
set -euo pipefail

AISTUDIO="${AISTUDIO:-/media/ai-rtx6000/AI_DATA/AiStudio}"
MOUNT="${MOUNT:-/media/ai-rtx6000/AI_DATA}"
DATA_ROOT="$AISTUDIO/data/factory"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
UV="${UV:-$HOME/.local/bin/uv}"
WITH_CAPTIONER=0
[[ "${1:-}" == "--captioner" ]] && WITH_CAPTIONER=1

src="$(findmnt -no SOURCE "$MOUNT" || true)"
fstype="$(findmnt -no FSTYPE "$MOUNT" || true)"
if [[ "$src" != "/dev/nvme1n1p1" || "$fstype" != "ext4" ]]; then
  echo "LỖI: $MOUNT không mount như mong đợi (thấy '$src' '$fstype'). Dừng, không ghi gì." >&2
  exit 1
fi
[[ "$HERE" == "$AISTUDIO/factory" ]] || { echo "LỖI: chạy từ $AISTUDIO/factory (đang ở $HERE)" >&2; exit 1; }
command -v ffmpeg >/dev/null || { echo "LỖI: thiếu ffmpeg" >&2; exit 1; }
[[ -x "$UV" ]] || { echo "LỖI: không thấy uv tại $UV" >&2; exit 1; }

export UV_CACHE_DIR="$AISTUDIO/cache/uv" UV_PYTHON_INSTALL_DIR="$AISTUDIO/cache/python"
export HF_HOME="$AISTUDIO/cache/huggingface"

cd "$HERE"
[[ -d .venv ]] || "$UV" venv --python 3.12 .venv
if [[ $WITH_CAPTIONER == 1 ]]; then
  "$UV" pip install --python .venv/bin/python --index-url https://download.pytorch.org/whl/cu128 torch
  "$UV" pip install --python .venv/bin/python -e ".[dev,analysis,captioner]"
else
  "$UV" pip install --python .venv/bin/python -e ".[dev,analysis]"
fi

mkdir -p "$DATA_ROOT" "$AISTUDIO/data/import/lofi" "$AISTUDIO/data/eval" "$AISTUDIO/models/captioner"
if [[ ! -f configs/project.yaml ]]; then
  cat > configs/project.yaml <<EOF
# Tạo bởi install_rtx6000.sh. Không commit.
data_root: $DATA_ROOT
acestep:
  repo_dir: $AISTUDIO/ACE-Step-1.5
  python: $AISTUDIO/ACE-Step-1.5/.venv/bin/python
  commit: ca1e85fe9430179831e6bc6be790c332190a3866
  checkpoints_dir: $AISTUDIO/ACE-Step-1.5/checkpoints
EOF
fi

.venv/bin/python scripts/doctor.py || true
.venv/bin/python -m pytest -q
echo "OK: factory cài tại $HERE, dữ liệu tại $DATA_ROOT"
