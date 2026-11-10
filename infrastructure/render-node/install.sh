#!/usr/bin/env bash
# Install or update the worker on a render node. Run as root ON THE RENDER NODE.
#   install.sh <path-to-repo-checkout>
# Only the worker code and its virtualenv are replaced. Models, LoRAs, datasets, checkpoints and
# /etc/lush/worker.env are never touched. This script does not install GPU drivers, ACE-Step or
# open any port.
set -euo pipefail

REPO="${1:?usage: install.sh <path-to-repo-checkout>}"
TARGET=/opt/lush-worker

id lush >/dev/null 2>&1 || useradd --system --create-home --shell /usr/sbin/nologin lush
mkdir -p "$TARGET" /etc/lush /var/lib/lush-worker/work /var/lib/lush-worker/loras

rsync -a --delete --exclude .venv --exclude .env --exclude __pycache__ --exclude .pytest_cache \
	"$REPO/workers/music-worker/" "$TARGET/"
[ -d "$TARGET/.venv" ] || python3 -m venv "$TARGET/.venv"
"$TARGET/.venv/bin/pip" install --quiet --upgrade pip
"$TARGET/.venv/bin/pip" install --quiet -e "$TARGET"

if [ ! -f /etc/lush/worker.env ]; then
	install -m 600 -o lush -g lush "$TARGET/.env.example" /etc/lush/worker.env
	echo "created /etc/lush/worker.env — set CONTROL_PLANE_URL and NODE_TOKEN before starting"
fi
chown -R lush:lush "$TARGET" /var/lib/lush-worker

install -m 644 "$REPO/infrastructure/render-node/lush-worker.service" /etc/systemd/system/lush-worker.service
systemctl daemon-reload
systemctl enable lush-worker
echo "installed. Start with: systemctl restart lush-worker && journalctl -u lush-worker -f"
