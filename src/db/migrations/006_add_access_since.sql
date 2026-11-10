-- Access-since tracking on the share join tables (idempotent upgrade path
-- for databases where 004_recipes.sql already created them without the column).
ALTER TABLE recipe_editors ADD COLUMN IF NOT EXISTS added_at TIMESTAMPTZ NOT NULL DEFAULT now();
ALTER TABLE recipe_viewers ADD COLUMN IF NOT EXISTS added_at TIMESTAMPTZ NOT NULL DEFAULT now();
