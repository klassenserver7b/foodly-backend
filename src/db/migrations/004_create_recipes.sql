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
