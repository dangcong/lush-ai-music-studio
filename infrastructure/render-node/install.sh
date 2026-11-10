#!/usr/bin/env bash
# Install or update the worker on a render node, as the normal user (no sudo).
#   install.sh <path-to-repo-checkout>
# Replaces only ~/lush-worker (code + virtualenv) and the systemd user units. Never touches models, LoRAs,
# datasets, checkpoints, ~/ACE-Step-1.5 or the config files in ~/.config/lush once they exist.
# Requires: uv (~/.local/bin/uv), lingering enabled for the user (one-time: sudo loginctl enable-linger $USER).
set -euo pipefail

REPO="${1:?usage: install.sh <path-to-repo-checkout>}"
TARGET="$HOME/lush-worker"
CONFIG="$HOME/.config/lush"
UNITS="$HOME/.config/systemd/user"
UV="$(command -v uv || echo "$HOME/.local/bin/uv")"

mkdir -p "$TARGET" "$CONFIG" "$UNITS" "$HOME/lush-data/work" "$HOME/lush-data/loras"
chmod 700 "$CONFIG"

rsync -a --delete --exclude .venv --exclude .env --exclude __pycache__ --exclude .pytest_cache \
	"$REPO/workers/music-worker/" "$TARGET/"
[ -d "$TARGET/.venv" ] || "$UV" venv --quiet --python 3.12 "$TARGET/.venv"
"$UV" pip install --quiet --python "$TARGET/.venv/bin/python" -e "$TARGET"

if [ ! -f "$CONFIG/worker.env" ]; then
	install -m 600 "$TARGET/.env.example" "$CONFIG/worker.env"
	echo "created $CONFIG/worker.env — set CONTROL_PLANE_URL and NODE_TOKEN before starting"
fi

install -m 644 "$REPO/infrastructure/render-node/lush-worker.service" "$UNITS/lush-worker.service"
install -m 644 "$REPO/infrastructure/render-node/acestep-api.service" "$UNITS/acestep-api.service"
systemctl --user daemon-reload
systemctl --user enable lush-worker.service >/dev/null
echo "installed. Start with: systemctl --user restart lush-worker && journalctl --user -u lush-worker -f"
