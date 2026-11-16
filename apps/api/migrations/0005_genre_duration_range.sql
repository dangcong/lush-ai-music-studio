-- Genres and styles (style presets) suggest a duration range (e.g. 60–120 s) instead of one value.
-- Both ends NULL = no suggestion of its own; the form then falls back style -> subgenre -> genre.
-- Existing rows keep their behaviour: both ends start at the old single value.
-- `genres.default_duration` and `style_presets.duration` stay in the tables but are no longer
-- read or written.
ALTER TABLE genres ADD COLUMN duration_min INTEGER;
ALTER TABLE genres ADD COLUMN duration_max INTEGER;
UPDATE genres SET duration_min = default_duration, duration_max = default_duration;

ALTER TABLE style_presets ADD COLUMN duration_min INTEGER;
ALTER TABLE style_presets ADD COLUMN duration_max INTEGER;
UPDATE style_presets SET duration_min = duration, duration_max = duration WHERE duration IS NOT NULL;
