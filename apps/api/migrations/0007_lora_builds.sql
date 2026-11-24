-- LoRA Studio P2: a LoRA built from a catalog trains from an immutable snapshot of the selection.

CREATE TABLE dataset_snapshots (
  id TEXT PRIMARY KEY, catalog_id TEXT NOT NULL REFERENCES catalogs(id),
  job_id TEXT NOT NULL UNIQUE REFERENCES jobs(id), lora_key TEXT NOT NULL UNIQUE,
  content TEXT NOT NULL, content_sha256 TEXT NOT NULL,
  created_by TEXT NOT NULL REFERENCES users(id), created_at TEXT NOT NULL);
CREATE TRIGGER dataset_snapshots_immutable BEFORE UPDATE ON dataset_snapshots
BEGIN SELECT RAISE(ABORT, 'dataset snapshot is immutable'); END;
ALTER TABLE loras ADD COLUMN snapshot_id TEXT REFERENCES dataset_snapshots(id);
CREATE TABLE lora_checkpoints (
  id TEXT PRIMARY KEY, lora_id TEXT NOT NULL REFERENCES loras(id),
  epoch INTEGER NOT NULL, step INTEGER, node_path TEXT NOT NULL, sha256 TEXT NOT NULL,
  bytes INTEGER, status TEXT NOT NULL DEFAULT 'kept' CHECK (status IN ('kept','chosen','deleted')),
  expires_at TEXT, note TEXT NOT NULL DEFAULT '', created_at TEXT NOT NULL,
  UNIQUE (lora_id, epoch));
