#!/usr/bin/env bash
# Install or update the worker on a render node, as the normal user (no sudo).
#   install.sh <path-to-repo-checkout>
# Everything lives under LUSH_ROOT (default /mnt/ai-studio/AiStudio, the dedicated NVMe):
#   lush-worker/  ACE-Step-1.5/  config/  data/{work,loras,datasets,training}  cache/  src/
# Replaces only LUSH_ROOT/lush-worker (code + virtualenv) and the systemd user units. Never touches models,
# LoRAs, datasets, checkpoints, ACE-Step-1.5, or config files that already exist.
# Requires: uv (~/.local/bin/uv), lingering enabled (one-time: sudo loginctl enable-linger $USER),
# and LUSH_ROOT on a mounted, user-writable disk.
set -euo pipefail

REPO="${1:?usage: install.sh <path-to-repo-checkout>}"
LUSH_ROOT="${LUSH_ROOT:-/mnt/ai-studio/AiStudio}"
TARGET="$LUSH_ROOT/lush-worker"
CONFIG="$LUSH_ROOT/config"
UNITS="$HOME/.config/systemd/user"
UV="$(command -v uv || echo "$HOME/.local/bin/uv")"
export UV_CACHE_DIR="$LUSH_ROOT/cache/uv"
export UV_PYTHON_INSTALL_DIR="$LUSH_ROOT/cache/python"

if [ ! -w "$LUSH_ROOT" ]; then
	echo "$LUSH_ROOT is missing or not writable. Mount the data disk first (see docs/RENDER_NODE_DEPLOYMENT.md)." >&2
	exit 1
fi

mkdir -p "$TARGET" "$CONFIG" "$UNITS" "$LUSH_ROOT"/data/{work,loras,datasets,training} "$LUSH_ROOT/cache"
chmod 700 "$CONFIG"

rsync -a --delete --exclude .venv --exclude .env --exclude __pycache__ --exclude .pytest_cache \
	"$REPO/workers/music-worker/" "$TARGET/"
[ -d "$TARGET/.venv" ] || "$UV" venv --quiet --python 3.12 "$TARGET/.venv"
"$UV" pip install --quiet --python "$TARGET/.venv/bin/python" -e "$TARGET"

if [ ! -f "$CONFIG/worker.env" ]; then
	install -m 600 "$TARGET/.env.example" "$CONFIG/worker.env"
	echo "created $CONFIG/worker.env — set CONTROL_PLANE_URL and NODE_TOKEN before starting"
fi

# Unit files carry the placeholder @LUSH_ROOT@.
for unit in lush-worker.service acestep-api.service; do
	sed "s|@LUSH_ROOT@|$LUSH_ROOT|g" "$REPO/infrastructure/render-node/$unit" > "$UNITS/$unit"
	chmod 644 "$UNITS/$unit"
done
systemctl --user daemon-reload
systemctl --user enable lush-worker.service >/dev/null
echo "installed under $LUSH_ROOT. Start with: systemctl --user restart lush-worker && journalctl --user -u lush-worker -f"
