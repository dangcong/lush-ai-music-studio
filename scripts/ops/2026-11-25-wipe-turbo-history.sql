-- One-off (D36, plan docs/superpowers/plans/2026-11-25-acestep-xl-only.md Task 8): wipe the generation
-- history made with ACE-Step Turbo and the Turbo model row. Not a migration: run once by hand with lush-api
-- stopped, after `.backup`. Keeps users, sessions, genres, nodes, captioner/scorer models and the XL row.
-- Every table that references jobs / tracks / artifacts other than these was empty on 2026-11-25
-- (loras, catalogs, caption_revisions, dataset_snapshots, lora_checkpoints, lora_eval_renders).
PRAGMA foreign_keys = ON;
BEGIN;
DELETE FROM artifacts;
UPDATE tracks SET parent_track_id = NULL;
DELETE FROM tracks;
DELETE FROM training_metrics;
DELETE FROM jobs;
DELETE FROM audit_logs;
DELETE FROM model_installations WHERE model_id IN (SELECT id FROM models WHERE key = 'acestep-v15-turbo');
DELETE FROM models WHERE key = 'acestep-v15-turbo';
COMMIT;
SELECT 'jobs', count(*) FROM jobs UNION ALL SELECT 'tracks', count(*) FROM tracks
UNION ALL SELECT 'artifacts', count(*) FROM artifacts UNION ALL SELECT 'audit_logs', count(*) FROM audit_logs
UNION ALL SELECT 'users', count(*) FROM users UNION ALL SELECT 'genres', count(*) FROM genres
UNION ALL SELECT 'models', count(*) FROM models UNION ALL SELECT 'nodes', count(*) FROM nodes;
PRAGMA foreign_key_check;
