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
# Release files belong to root and are world-readable: the service user only needs to read and execute them.
tar --no-same-owner --no-same-permissions -xzf "$ARCHIVE" -C "$RELEASE"
chown -R root:root "$RELEASE"
chmod -R u=rwX,go=rX "$RELEASE"
chmod 755 "$RELEASE/lush-api"

PREVIOUS=""
if [ -L /opt/lush/current ] && [ -d "$(readlink -f /opt/lush/current)" ]; then
	PREVIOUS="$(readlink -f /opt/lush/current)"
fi
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

journalctl -u lush-api -n 20 --no-pager -o cat >&2 || true
if [ -n "$PREVIOUS" ]; then
	echo "health check failed; rolling back to $PREVIOUS" >&2
	ln -sfn "$PREVIOUS" /opt/lush/current
	systemctl restart lush-api
else
	echo "health check failed on first deploy; stopping the service" >&2
	systemctl stop lush-api
fi
exit 1
