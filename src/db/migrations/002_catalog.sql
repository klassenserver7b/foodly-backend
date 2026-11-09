-- Catalog tables: tags and ingredients.
--
-- Tag ids are the display name itself, so recipes can reference them by text
-- without an extra join. Ingredients get a proper identity id.

CREATE TABLE IF NOT EXISTS tags (
    id  TEXT PRIMARY KEY,    -- the display name IS the id (e.g. 'Hauptgericht')
    svg TEXT                 -- hash/filename of optional SVG icon; NULL = no icon
);

CREATE TABLE IF NOT EXISTS ingredients (
    id   BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name TEXT NOT NULL
);
