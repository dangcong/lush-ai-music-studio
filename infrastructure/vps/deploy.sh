#!/usr/bin/env bash
# Install a web release on the VPS. Run as root ON THE VPS, never from CI without approval.
#   deploy.sh <release.tar.gz>
# The archive contains: lush-api (binary) and web/ (built React app).
# Data in /var/lib/lush is never touched; the previous release stays in /opt/lush/releases for rollback.
set -euo pipefail

ARCHIVE="${1:?usage: deploy.sh <release.tar.gz>}"
RELEASE="/opt/lush/releases/$(date -u +%Y%m%d-%H%M%S)"
# Health URL follows BIND_ADDR in the service's environment file.
BIND_ADDR="$(sed -n 's/^BIND_ADDR=//p' /etc/lush/api.env | tail -1)"
HEALTH_URL="http://${BIND_ADDR:-127.0.0.1:8080}/api/health"

/opt/lush/backup.sh || { echo "backup failed; aborting" >&2; exit 1; }

mkdir -p "$RELEASE"
tar -xzf "$ARCHIVE" -C "$RELEASE"
chmod +x "$RELEASE/lush-api"

PREVIOUS="$(readlink -f /opt/lush/current || true)"
ln -sfn "$RELEASE" /opt/lush/current
systemctl restart lush-api

# Health check; roll back automatically when the new release does not come up.
for _ in $(seq 1 20); do
	if curl -fsS "$HEALTH_URL" >/dev/null; then
		echo "deployed $RELEASE"
		exit 0
	fi
	sleep 1
done

echo "health check failed; rolling back to $PREVIOUS" >&2
if [ -n "$PREVIOUS" ] && [ -d "$PREVIOUS" ]; then
	ln -sfn "$PREVIOUS" /opt/lush/current
	systemctl restart lush-api
fi
exit 1
