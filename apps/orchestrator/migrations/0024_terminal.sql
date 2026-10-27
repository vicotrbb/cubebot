CREATE TABLE terminal_invocations (
    id uuid PRIMARY KEY,
    turn_id uuid NOT NULL REFERENCES chat_turns(id),
    authority_id uuid NOT NULL REFERENCES chat_authorities(id),
    request jsonb NOT NULL,
    expires_at timestamptz NOT NULL,
    outcome jsonb,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX terminal_turn ON terminal_invocations(turn_id,created_at);
ALTER TABLE chat_turns DROP CONSTRAINT chat_turns_status_check;
ALTER TABLE chat_turns ADD CHECK (status IN ('queued','running','retrying','waiting','complete','cancelled'));
ALTER TABLE chat_authorities ADD COLUMN terminal_budget smallint NOT NULL DEFAULT 0 CHECK (terminal_budget IN (0,8));
