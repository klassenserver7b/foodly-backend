CREATE TABLE IF NOT EXISTS images (
    id   BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    hash TEXT NOT NULL UNIQUE,   -- content hash = filename on disk
    name TEXT                     -- optional display name
);
