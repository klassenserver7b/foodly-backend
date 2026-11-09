-- Recipe domain: recipes, sections, steps, and per-recipe ingredient lines.

CREATE TABLE IF NOT EXISTS recipes (
    id               BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    owner_id         BIGINT      NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name             TEXT        NOT NULL,
    source           TEXT,
    time_display     TEXT,                        -- human-readable (e.g. "45 min {Kochzeit}")
    work_minutes     INT,
    overall_minutes  INT,
    size_number      INT,                         -- editable portion count; NULL = fixed descriptor
    size_text        TEXT,                         -- TagText label (e.g. "{Portionen}")
    notes            TEXT[]      NOT NULL DEFAULT '{}',
    main_image_id    BIGINT      REFERENCES images(id) ON DELETE SET NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_recipes_owner ON recipes(owner_id);

CREATE TABLE IF NOT EXISTS recipe_tags (
    recipe_id  BIGINT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    tag_id     TEXT   NOT NULL REFERENCES tags(id) ON DELETE CASCADE,
    position   INT    NOT NULL DEFAULT 0,

    PRIMARY KEY (recipe_id, tag_id)
);

CREATE TABLE IF NOT EXISTS recipe_images (
    recipe_id BIGINT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    image_id  BIGINT NOT NULL REFERENCES images(id) ON DELETE CASCADE,
    position  INT    NOT NULL DEFAULT 0,

    PRIMARY KEY (recipe_id, image_id)
);

CREATE TABLE IF NOT EXISTS sections (
    id        BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    recipe_id BIGINT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    name      TEXT,
    position  INT    NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_sections_recipe ON sections(recipe_id);

CREATE TABLE IF NOT EXISTS steps (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    section_id BIGINT NOT NULL REFERENCES sections(id) ON DELETE CASCADE,
    text       TEXT   NOT NULL,          -- the instruction text
    position   INT    NOT NULL DEFAULT 0
    -- future: duration INT  (seconds, for recipe-linked timers)
 -- future: duration INT  (seconds, for recipe-linked timers)
);

CREATE INDEX IF NOT EXISTS idx_steps_section ON steps(section_id);

CREATE TABLE IF NOT EXISTS recipe_ingredients (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    section_id    BIGINT NOT NULL REFERENCES sections(id) ON DELETE CASCADE,
    ingredient_id BIGINT REFERENCES ingredients(id) ON DELETE SET NULL,  -- NULL = freetext line
    text          TEXT,                 -- suffix after ingredient name, OR standalone freetext
    amount        TEXT,                 -- quantity as string (e.g. "500", "2-3")
    amount_prefix TEXT,                 -- prefix before amount (e.g. "ca.")
    unit          TEXT,                 -- unit string (e.g. "g", "EL")
    position      INT    NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_recipe_ingredients_section ON recipe_ingredients(section_id);

-- Sharing and visibility

CREATE TABLE IF NOT EXISTS recipe_editors (
    recipe_id BIGINT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    user_id   BIGINT NOT NULL REFERENCES users(id)   ON DELETE CASCADE,

    PRIMARY KEY (recipe_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_recipe_editors_user ON recipe_editors(user_id);

CREATE TABLE IF NOT EXISTS recipe_viewers (
    recipe_id BIGINT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    user_id   BIGINT NOT NULL REFERENCES users(id)   ON DELETE CASCADE,

    PRIMARY KEY (recipe_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_recipe_viewers_user ON recipe_viewers(user_id);

-- Ratings

CREATE TABLE IF NOT EXISTS user_ratings (
    recipe_id BIGINT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    user_id   BIGINT NOT NULL REFERENCES users(id)   ON DELETE CASCADE,
    rating    SMALLINT NOT NULL CHECK (rating >= 1 AND rating <= 5),

    PRIMARY KEY (recipe_id, user_id)
);
