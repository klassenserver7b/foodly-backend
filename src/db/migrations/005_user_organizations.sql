-- User-side organization: personal categories and shareable groups.

CREATE TABLE IF NOT EXISTS user_categories (
    id          BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    user_id     BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    name        TEXT   NOT NULL,
    sort_order  INT,                         -- position among the user's categories
    color       TEXT   NOT NULL,             -- primary hex color (e.g. "#e11d48")
    color_light TEXT,                        -- optional light-mode override
    color_dark  TEXT                         -- optional dark-mode override
);

CREATE INDEX IF NOT EXISTS idx_user_categories_user ON user_categories(user_id);

CREATE TABLE IF NOT EXISTS user_category_recipes (
    category_id BIGINT NOT NULL REFERENCES user_categories(id) ON DELETE CASCADE,
    recipe_id   BIGINT NOT NULL REFERENCES recipes(id)         ON DELETE CASCADE,
    position    INT    NOT NULL DEFAULT 0,

    PRIMARY KEY (category_id, recipe_id)
);

CREATE TABLE IF NOT EXISTS groups (
    id        BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    name      TEXT   NOT NULL,
    owner_id  BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_groups_owner ON groups(owner_id);

CREATE TABLE IF NOT EXISTS group_members (
    group_id BIGINT NOT NULL REFERENCES groups(id) ON DELETE CASCADE,
    user_id  BIGINT NOT NULL REFERENCES users(id)  ON DELETE CASCADE,

    PRIMARY KEY (group_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_group_members_user ON group_members(user_id);
