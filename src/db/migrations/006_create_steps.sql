CREATE TABLE IF NOT EXISTS steps (
    id         BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    section_id BIGINT NOT NULL REFERENCES sections(id) ON DELETE CASCADE,
    text       TEXT   NOT NULL,          -- the instruction text
    position   INT    NOT NULL DEFAULT 0
    -- future: duration INT  (seconds, for recipe-linked timers)
);

CREATE INDEX IF NOT EXISTS idx_steps_section ON steps(section_id);
