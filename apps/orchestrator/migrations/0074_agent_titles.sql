ALTER TABLE agents
    ADD COLUMN title text NOT NULL DEFAULT '' CHECK (title = btrim(title) AND length(title) <= 64);
