-- AI Training (M1): Style LoRA runs are jobs of type 'train_lora' (docs/STYLE_TRAINING.md).

-- Where a LoRA came from. Filled when its training job completes.
ALTER TABLE loras ADD COLUMN job_id TEXT REFERENCES jobs(id);
ALTER TABLE loras ADD COLUMN dataset_key TEXT;
ALTER TABLE loras ADD COLUMN trainer TEXT;
ALTER TABLE loras ADD COLUMN config TEXT NOT NULL DEFAULT '{}';
ALTER TABLE loras ADD COLUMN adapter_sha256 TEXT;
ALTER TABLE loras ADD COLUMN created_by TEXT REFERENCES users(id);

-- Dataset versions a render node reports at registration. The audio never leaves the node (D9).
CREATE TABLE node_datasets (
    node_id TEXT NOT NULL REFERENCES nodes(id) ON DELETE CASCADE,
    key     TEXT NOT NULL,
    info    TEXT NOT NULL DEFAULT '{}',
    PRIMARY KEY (node_id, key)
);

-- Structured training progress, exactly as the trainer reported it. A value the trainer does not
-- provide stays NULL; nothing here is interpolated.
CREATE TABLE training_metrics (
    job_id          TEXT NOT NULL REFERENCES jobs(id) ON DELETE CASCADE,
    sequence        INTEGER NOT NULL,
    global_step     INTEGER,
    epoch           INTEGER,
    train_loss      REAL,
    learning_rate   REAL,
    elapsed_seconds REAL,
    gpu_memory_mb   INTEGER,
    recorded_at     TEXT NOT NULL,
    PRIMARY KEY (job_id, sequence)
);
