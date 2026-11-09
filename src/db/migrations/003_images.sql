-- Content-addressed image storage.
--
-- `hash` is the SHA-256 of the file bytes and doubles as the on-disk filename
-- (sharded by the first two hex pairs). `owner_id` records who uploaded the
-- image so only the owner can delete it.

CREATE TABLE IF NOT EXISTS images (
    id       BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    hash     TEXT NOT NULL UNIQUE,   -- content hash = filename on disk
    name     TEXT,                   -- optional display name
    owner_id BIGINT REFERENCES users(id)
);

CREATE INDEX IF NOT EXISTS idx_images_owner_id ON images(owner_id);
