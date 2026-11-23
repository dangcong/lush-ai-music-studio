-- Catalog: uploaded songs plus captions per captioner model. `model_key` stores models.key (not id)
-- so captions survive a model re-register.
ALTER TABLE models ADD COLUMN kind TEXT NOT NULL DEFAULT 'generator'
  CHECK (kind IN ('generator','captioner','scorer'));

CREATE TABLE catalogs (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, genre_id TEXT NOT NULL REFERENCES genres(id),
  created_by TEXT NOT NULL REFERENCES users(id), created_at TEXT NOT NULL, updated_at TEXT NOT NULL,
  deleted_at TEXT);
CREATE UNIQUE INDEX catalogs_name_live ON catalogs(name) WHERE deleted_at IS NULL;

CREATE TABLE catalog_tracks (
  id TEXT PRIMARY KEY, catalog_id TEXT NOT NULL REFERENCES catalogs(id),
  artifact_id TEXT NOT NULL REFERENCES artifacts(id), filename TEXT NOT NULL, sha256 TEXT NOT NULL,
  duration_s REAL, tempo_bpm REAL, tempo_source TEXT CHECK (tempo_source IN ('auto','human')),
  key_name TEXT, key_source TEXT CHECK (key_source IN ('auto','human')),
  analysis TEXT NOT NULL DEFAULT '{}',
  analysis_status TEXT NOT NULL DEFAULT 'pending' CHECK (analysis_status IN ('pending','done','failed')),
  analysis_job_id TEXT REFERENCES jobs(id),
  current_model_key TEXT,
  created_by TEXT NOT NULL REFERENCES users(id), created_at TEXT NOT NULL, deleted_at TEXT);
CREATE UNIQUE INDEX catalog_tracks_sha_live ON catalog_tracks(catalog_id, sha256) WHERE deleted_at IS NULL;

CREATE TABLE caption_revisions (
  id TEXT PRIMARY KEY, catalog_track_id TEXT NOT NULL REFERENCES catalog_tracks(id),
  model_key TEXT NOT NULL, seq INTEGER NOT NULL,
  mood TEXT NOT NULL DEFAULT '', instruments TEXT NOT NULL DEFAULT '', vocals TEXT NOT NULL DEFAULT '',
  caption TEXT NOT NULL DEFAULT '',
  source TEXT NOT NULL CHECK (source IN ('model','human')), source_ref TEXT NOT NULL DEFAULT '',
  job_id TEXT REFERENCES jobs(id), created_by TEXT REFERENCES users(id), created_at TEXT NOT NULL,
  UNIQUE (catalog_track_id, model_key, seq));
CREATE UNIQUE INDEX caption_revisions_job_track ON caption_revisions(job_id, catalog_track_id) WHERE job_id IS NOT NULL;

CREATE TABLE caption_heads (
  catalog_track_id TEXT NOT NULL REFERENCES catalog_tracks(id), model_key TEXT NOT NULL,
  current_seq INTEGER NOT NULL, PRIMARY KEY (catalog_track_id, model_key));
