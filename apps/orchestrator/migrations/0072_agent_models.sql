ALTER TABLE agents
    ADD COLUMN model_provider text,
    ADD COLUMN model text CHECK (length(btrim(model)) BETWEEN 1 AND 200),
    ADD COLUMN model_context_window bigint CHECK (model_context_window BETWEEN 1024 AND 100000000),
    ADD CHECK ((model_provider IS NULL) = (model IS NULL)),
    ADD CHECK (model IS NOT NULL OR model_context_window IS NULL);
