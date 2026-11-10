#!/usr/bin/env bash
# Consistent online backup of the SQLite database (safe while lush-api is running).
# Artifacts (audio files) are plain files: back up /var/lib/lush/artifacts with rsync/restic separately.
set -euo pipefail

DATA_DIR="${DATA_DIR:-/var/lib/lush}"
BACKUP_DIR="${BACKUP_DIR:-/var/backups/lush}"
KEEP_DAYS="${KEEP_DAYS:-14}"

if [ ! -f "$DATA_DIR/lush.db" ]; then
	echo "no database yet at $DATA_DIR/lush.db; nothing to back up"
	exit 0
fi

mkdir -p "$BACKUP_DIR"
TARGET="$BACKUP_DIR/lush-$(date -u +%Y%m%d-%H%M%S).db"
sqlite3 "$DATA_DIR/lush.db" ".backup '$TARGET'"
gzip "$TARGET"
find "$BACKUP_DIR" -name 'lush-*.db.gz' -mtime +"$KEEP_DAYS" -delete
echo "backup written: $TARGET.gz"
