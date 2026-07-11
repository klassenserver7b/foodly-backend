CREATE TABLE IF NOT EXISTS recipe_ingredients (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    section_id    BIGINT NOT NULL REFERENCES sections(id) ON DELETE CASCADE,
    ingredient_id BIGINT REFERENCES ingredients(id) ON DELETE SET NULL,  -- NULL = freetext line
    text          TEXT,               -- suffix after ingredient name, OR standalone freetext
    amount        TEXT,               -- quantity as string (e.g. "500", "2-3")
    amount_prefix TEXT,               -- prefix before amount (e.g. "ca.")
    unit          TEXT,               -- unit string (e.g. "g", "EL")
    position      INT    NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_recipe_ingredients_section ON recipe_ingredients(section_id);
