-- Core schema. All ids are opaque TEXT (UUID); timestamps are UTC 'YYYY-MM-DDTHH:MM:SS.sssZ'.

CREATE TABLE users (
    id            TEXT PRIMARY KEY,
    username      TEXT NOT NULL UNIQUE COLLATE NOCASE,
    display_name  TEXT NOT NULL DEFAULT '',
    password_hash TEXT NOT NULL,
    role          TEXT NOT NULL CHECK (role IN ('admin', 'trainer', 'member')),
    is_active     INTEGER NOT NULL DEFAULT 1,
    created_at    TEXT NOT NULL
);

CREATE TABLE sessions (
    token_hash TEXT PRIMARY KEY,
    user_id    TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL
);

CREATE TABLE projects (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    owner_id    TEXT NOT NULL REFERENCES users(id),
    created_at  TEXT NOT NULL
);

-- Genre registry. A subgenre is a genre with parent_id (two levels only).
CREATE TABLE genres (
    id               TEXT PRIMARY KEY,
    parent_id        TEXT REFERENCES genres(id),
    name             TEXT NOT NULL,
    slug             TEXT NOT NULL UNIQUE,
    description      TEXT NOT NULL DEFAULT '',
    prompt_template  TEXT NOT NULL DEFAULT '',
    instruments      TEXT NOT NULL DEFAULT '[]',
    moods            TEXT NOT NULL DEFAULT '[]',
    bpm_min          INTEGER,
    bpm_max          INTEGER,
    default_duration INTEGER NOT NULL DEFAULT 120,
    is_active        INTEGER NOT NULL DEFAULT 1,
    sort_order       INTEGER NOT NULL DEFAULT 0
);

-- Model registry (metadata only; weights live on render nodes).
CREATE TABLE models (
    id           TEXT PRIMARY KEY,
    engine       TEXT NOT NULL,
    key          TEXT NOT NULL,
    name         TEXT NOT NULL,
    version      TEXT NOT NULL DEFAULT '',
    description  TEXT NOT NULL DEFAULT '',
    capabilities TEXT NOT NULL DEFAULT '[]',
    status       TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    created_at   TEXT NOT NULL,
    UNIQUE (engine, key)
);

-- LoRA registry. Training metadata is added in Phase 3.
CREATE TABLE loras (
    id            TEXT PRIMARY KEY,
    key           TEXT NOT NULL UNIQUE,
    name          TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    genre_id      TEXT REFERENCES genres(id),
    base_model_id TEXT NOT NULL REFERENCES models(id),
    status        TEXT NOT NULL DEFAULT 'evaluation'
                  CHECK (status IN ('training', 'evaluation', 'approved', 'archived')),
    created_at    TEXT NOT NULL
);

CREATE TABLE style_presets (
    id              TEXT PRIMARY KEY,
    genre_id        TEXT NOT NULL REFERENCES genres(id),
    name            TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    kind            TEXT NOT NULL DEFAULT 'genre' CHECK (kind IN ('genre', 'trained')),
    prompt_template TEXT NOT NULL DEFAULT '',
    instruments     TEXT NOT NULL DEFAULT '[]',
    moods           TEXT NOT NULL DEFAULT '[]',
    bpm             INTEGER,
    duration        INTEGER,
    model_id        TEXT REFERENCES models(id),
    lora_id         TEXT REFERENCES loras(id),
    lora_scale      REAL,
    params          TEXT NOT NULL DEFAULT '{}',
    version         INTEGER NOT NULL DEFAULT 1,
    is_active       INTEGER NOT NULL DEFAULT 1
);

-- Render node registry. The latest heartbeat is kept on the row (no history in V1).
CREATE TABLE nodes (
    id               TEXT PRIMARY KEY,
    name             TEXT NOT NULL UNIQUE,
    token_hash       TEXT NOT NULL UNIQUE,
    status           TEXT NOT NULL DEFAULT 'active' CHECK (status IN ('active', 'disabled')),
    maintenance      INTEGER NOT NULL DEFAULT 0,
    worker_version   TEXT,
    protocol_version INTEGER,
    job_types        TEXT NOT NULL DEFAULT '[]',
    engines          TEXT NOT NULL DEFAULT '[]',
    gpu              TEXT NOT NULL DEFAULT '{}',
    last_seen        TEXT,
    registered_at    TEXT,
    created_at       TEXT NOT NULL
);

CREATE TABLE model_installations (
    node_id  TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    model_id TEXT NOT NULL REFERENCES models(id) ON DELETE CASCADE,
    PRIMARY KEY (node_id, model_id)
);

CREATE TABLE lora_installations (
    node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    lora_id TEXT NOT NULL REFERENCES loras(id) ON DELETE CASCADE,
    PRIMARY KEY (node_id, lora_id)
);

-- One durable queue for every job type (generate now; train/export later).
CREATE TABLE jobs (
    id               TEXT PRIMARY KEY,
    type             TEXT NOT NULL,
    status           TEXT NOT NULL DEFAULT 'queued'
                     CHECK (status IN ('queued', 'preparing', 'running', 'completed', 'failed', 'cancelled')),
    priority         INTEGER NOT NULL DEFAULT 0,
    owner_id         TEXT NOT NULL REFERENCES users(id),
    project_id       TEXT REFERENCES projects(id),
    title            TEXT NOT NULL DEFAULT '',
    request          TEXT NOT NULL DEFAULT '{}',
    payload          TEXT NOT NULL DEFAULT '{}',
    result           TEXT,
    model_id         TEXT REFERENCES models(id),
    lora_id          TEXT REFERENCES loras(id),
    progress         REAL NOT NULL DEFAULT 0,
    stage            TEXT,
    message          TEXT,
    log              TEXT NOT NULL DEFAULT '',
    error            TEXT,
    attempt          INTEGER NOT NULL DEFAULT 0,
    max_attempts     INTEGER NOT NULL DEFAULT 3,
    cancel_requested INTEGER NOT NULL DEFAULT 0,
    node_id          TEXT REFERENCES nodes(id),
    lease_token      TEXT,
    lease_expires_at TEXT,
    idempotency_key  TEXT,
    created_at       TEXT NOT NULL,
    started_at       TEXT,
    finished_at      TEXT,
    updated_at       TEXT NOT NULL
);
CREATE INDEX jobs_queue_idx ON jobs (status, priority DESC, created_at);
CREATE INDEX jobs_owner_idx ON jobs (owner_id, created_at);
CREATE UNIQUE INDEX jobs_idempotency_idx ON jobs (owner_id, idempotency_key) WHERE idempotency_key IS NOT NULL;

-- Music library. Versions of a track point at their root via parent_track_id.
CREATE TABLE tracks (
    id              TEXT PRIMARY KEY,
    title           TEXT NOT NULL,
    owner_id        TEXT NOT NULL REFERENCES users(id),
    project_id      TEXT REFERENCES projects(id),
    job_id          TEXT UNIQUE REFERENCES jobs(id),
    parent_track_id TEXT REFERENCES tracks(id),
    version         INTEGER NOT NULL DEFAULT 1,
    prompt          TEXT NOT NULL DEFAULT '',
    prompt_final    TEXT NOT NULL DEFAULT '',
    genre_id        TEXT REFERENCES genres(id),
    subgenre_id     TEXT REFERENCES genres(id),
    preset_id       TEXT REFERENCES style_presets(id),
    model_id        TEXT REFERENCES models(id),
    lora_id         TEXT REFERENCES loras(id),
    lora_scale      REAL,
    seed            INTEGER,
    bpm             INTEGER,
    key             TEXT,
    time_signature  TEXT,
    duration        REAL,
    favorite        INTEGER NOT NULL DEFAULT 0,
    rating          INTEGER NOT NULL DEFAULT 0,
    params          TEXT NOT NULL DEFAULT '{}',
    created_at      TEXT NOT NULL,
    deleted_at      TEXT
);
CREATE INDEX tracks_created_idx ON tracks (created_at);

-- Files are addressed by artifact id; storage_key is relative to DATA_DIR and never exposed.
CREATE TABLE artifacts (
    id           TEXT PRIMARY KEY,
    kind         TEXT NOT NULL,
    job_id       TEXT REFERENCES jobs(id),
    track_id     TEXT REFERENCES tracks(id),
    storage_key  TEXT NOT NULL,
    filename     TEXT NOT NULL,
    content_type TEXT NOT NULL,
    size_bytes   INTEGER NOT NULL,
    sha256       TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    UNIQUE (job_id, kind)
);
CREATE INDEX artifacts_track_idx ON artifacts (track_id);

CREATE TABLE audit_logs (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    actor_id    TEXT,
    action      TEXT NOT NULL,
    target_type TEXT NOT NULL DEFAULT '',
    target_id   TEXT NOT NULL DEFAULT '',
    detail      TEXT NOT NULL DEFAULT '{}',
    created_at  TEXT NOT NULL
);
