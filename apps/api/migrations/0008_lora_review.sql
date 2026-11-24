-- LoRA Studio P3: listening renders, scores and the chosen checkpoint.

ALTER TABLE lora_checkpoints ADD COLUMN delete_requested_at TEXT;
ALTER TABLE lora_checkpoints ADD COLUMN style_score REAL;
ALTER TABLE lora_checkpoints ADD COLUMN train_score REAL;
ALTER TABLE lora_checkpoints ADD COLUMN train_match TEXT;
ALTER TABLE lora_checkpoints ADD COLUMN melody_score REAL;
ALTER TABLE lora_checkpoints ADD COLUMN melody_match TEXT;
ALTER TABLE lora_checkpoints ADD COLUMN quality_score REAL;
ALTER TABLE loras ADD COLUMN chosen_checkpoint_id TEXT REFERENCES lora_checkpoints(id);
ALTER TABLE loras ADD COLUMN base_key TEXT;
CREATE TABLE lora_eval_renders (
  id TEXT PRIMARY KEY, lora_id TEXT NOT NULL REFERENCES loras(id),
  job_id TEXT NOT NULL REFERENCES jobs(id), checkpoint_id TEXT REFERENCES lora_checkpoints(id),
  kind TEXT NOT NULL CHECK (kind IN ('base','prompt','held_out')),
  sample_filename TEXT, prompt TEXT NOT NULL, artifact_id TEXT NOT NULL REFERENCES artifacts(id),
  scores TEXT NOT NULL DEFAULT '{}', created_at TEXT NOT NULL,
  UNIQUE (job_id, artifact_id));
