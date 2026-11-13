-- User-uploaded input audio (Reference Audio / Source Audio) is an artifact of kind 'audio_input'
-- that belongs to a user instead of a job or a track.
ALTER TABLE artifacts ADD COLUMN owner_id TEXT REFERENCES users(id);
