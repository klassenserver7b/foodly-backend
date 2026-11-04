CREATE TABLE IF NOT EXISTS sections (
    id        BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    recipe_id BIGINT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    name      TEXT,
    position  INT    NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_sections_recipe ON sections(recipe_id);
