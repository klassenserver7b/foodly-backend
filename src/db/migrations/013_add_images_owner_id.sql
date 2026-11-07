-- Track who uploaded each image so only the owner can delete it.
ALTER TABLE images ADD COLUMN IF NOT EXISTS owner_id BIGINT REFERENCES users(id);

-- Index for owner-scoped lookups (e.g. "list my images", owner-only DELETE).
CREATE INDEX IF NOT EXISTS idx_images_owner_id ON images(owner_id);
