-- D37: words that are unlikely in a genre's captions; the LoRA build preview warns when a caption uses one
-- (machine captioners sometimes invent details such as a brass section in a lo-fi track).
ALTER TABLE genres ADD COLUMN caption_watch_words TEXT NOT NULL DEFAULT '[]';

UPDATE genres SET caption_watch_words =
  '["brass","orchestral","orchestra","cinematic","epic","rap","rapping","choir","edm","dubstep","metal","distorted guitar"]'
WHERE slug = 'lo-fi' OR parent_id IN (SELECT id FROM genres WHERE slug = 'lo-fi');
