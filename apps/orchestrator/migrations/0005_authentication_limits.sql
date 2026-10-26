ALTER TABLE human_accounts
    ADD COLUMN auth_window_started_at timestamptz NOT NULL DEFAULT to_timestamp(0),
    ADD COLUMN auth_attempts integer NOT NULL DEFAULT 0 CHECK (auth_attempts BETWEEN 0 AND 10);
