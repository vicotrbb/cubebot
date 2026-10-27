CREATE TABLE chat_authorities (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    turn_id uuid NOT NULL REFERENCES chat_turns(id),
    attempt_id uuid NOT NULL REFERENCES agent_resources(id),
    provider_revision bigint NOT NULL,
    expires_at timestamptz NOT NULL DEFAULT clock_timestamp() + interval '60 seconds',
    outcome jsonb,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
ALTER TABLE chat_turns ADD COLUMN authority_id uuid REFERENCES chat_authorities(id);
