CREATE TABLE IF NOT EXISTS user_ratings (
    recipe_id BIGINT NOT NULL REFERENCES recipes(id) ON DELETE CASCADE,
    user_id   BIGINT NOT NULL REFERENCES users(id)   ON DELETE CASCADE,
    rating    SMALLINT NOT NULL CHECK (rating >= 1 AND rating <= 5),

    PRIMARY KEY (recipe_id, user_id)
);
