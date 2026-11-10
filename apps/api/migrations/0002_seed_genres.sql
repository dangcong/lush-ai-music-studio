-- Initial genre registry. Further genres are added through the Genre Manager UI, not by migrations.

INSERT INTO genres (id, parent_id, name, slug, description, prompt_template, instruments, moods, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), NULL, 'Lo-fi', 'lo-fi', 'Nhạc lo-fi thư giãn, ấm, hơi bụi.', 'lo-fi, mellow, warm tape saturation, soft vinyl crackle, relaxed groove', '["electric piano", "soft drums", "upright bass", "guitar", "vinyl crackle"]', '["chill", "warm", "nostalgic", "cozy", "dreamy"]', 60, 95, 120, 0);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'lo-fi'), 'Lo-fi Hip Hop', 'lo-fi-hip-hop', 'lo-fi hip hop, dusty boom-bap drums, mellow chords', 60, 95, 120, 0);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'lo-fi'), 'Chill Lo-fi', 'chill-lo-fi', 'chill lo-fi, laid-back beat, soft pads', 60, 95, 120, 1);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'lo-fi'), 'Lo-fi Jazz', 'lo-fi-jazz', 'lo-fi jazz, jazzy chords, brushed drums, warm tape sound', 60, 95, 120, 2);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'lo-fi'), 'Ambient Lo-fi', 'ambient-lo-fi', 'ambient lo-fi, airy textures, slow gentle beat', 60, 95, 120, 3);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'lo-fi'), 'Relaxing Piano', 'relaxing-piano', 'relaxing lo-fi piano, soft felt piano, gentle and intimate', 60, 95, 120, 4);

INSERT INTO genres (id, parent_id, name, slug, description, prompt_template, instruments, moods, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), NULL, 'Jazz', 'jazz', 'Jazz không lời, mượt và sang.', 'jazz instrumental, smooth, expressive, live band feel', '["piano", "upright bass", "brushed drums", "saxophone", "trumpet", "guitar"]', '["smooth", "elegant", "late night", "romantic", "relaxed"]', 70, 140, 120, 1);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'jazz'), 'Smooth Jazz', 'smooth-jazz', 'smooth jazz, silky saxophone lead, polished groove', 70, 140, 120, 0);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'jazz'), 'Piano Jazz', 'piano-jazz', 'piano jazz trio, expressive piano, walking bass', 70, 140, 120, 1);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'jazz'), 'Lounge Jazz', 'lounge-jazz', 'lounge jazz, laid-back, soft and sophisticated', 70, 140, 120, 2);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'jazz'), 'Soft Jazz', 'soft-jazz', 'soft jazz, gentle dynamics, warm tone', 70, 140, 120, 3);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'jazz'), 'Jazz Instrumental', 'jazz-instrumental', 'jazz instrumental, ensemble interplay, swing feel', 70, 140, 120, 4);

INSERT INTO genres (id, parent_id, name, slug, description, prompt_template, instruments, moods, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), NULL, 'Dark Ambient', 'dark-ambient', 'Không gian tối, chậm, nhiều lớp âm.', 'dark ambient, slow evolving textures, deep atmosphere, no drums', '["synth pads", "drones", "sub bass", "field recordings", "bowed strings"]', '["dark", "mysterious", "ominous", "vast", "cold"]', 40, 80, 120, 2);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'dark-ambient'), 'Dark Atmospheric', 'dark-atmospheric', 'dark atmospheric, brooding soundscape, distant echoes', 40, 80, 120, 0);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'dark-ambient'), 'Drone Ambient', 'drone-ambient', 'drone ambient, sustained low drones, minimal movement', 40, 80, 120, 1);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'dark-ambient'), 'Cinematic Ambient', 'cinematic-ambient', 'cinematic dark ambient, wide orchestral textures, tension', 40, 80, 120, 2);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'dark-ambient'), 'Minimal Dark Ambient', 'minimal-dark-ambient', 'minimal dark ambient, sparse, quiet, long tones', 40, 80, 120, 3);

INSERT INTO genres (id, parent_id, name, slug, description, prompt_template, instruments, moods, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), NULL, 'Dark Academia', 'dark-academia', 'Piano và dây cổ điển, trầm buồn, học thuật.', 'dark academia, neo-classical, melancholic, intimate chamber sound', '["piano", "cello", "violin", "string ensemble", "harp"]', '["melancholic", "nostalgic", "scholarly", "romantic", "somber"]', 55, 110, 120, 3);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'dark-academia'), 'Dark Academia Piano', 'dark-academia-piano', 'dark academia solo piano, melancholic, candlelit mood', 55, 110, 120, 0);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'dark-academia'), 'Neo-classical', 'neo-classical', 'neo-classical, modern classical composition, emotive', 55, 110, 120, 1);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'dark-academia'), 'Melancholic Piano', 'melancholic-piano', 'melancholic piano, slow, sorrowful melody', 55, 110, 120, 2);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'dark-academia'), 'Piano and Strings', 'piano-and-strings', 'piano and strings, cello and violin accompaniment, tender', 55, 110, 120, 3);
INSERT INTO genres (id, parent_id, name, slug, prompt_template, bpm_min, bpm_max, default_duration, sort_order) VALUES
  (lower(hex(randomblob(16))), (SELECT id FROM genres WHERE slug = 'dark-academia'), 'Cinematic Classical', 'cinematic-classical', 'cinematic classical, sweeping strings, dramatic build', 55, 110, 120, 4);
