CREATE TABLE conversations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id uuid NOT NULL REFERENCES agents(id),
    created_by uuid NOT NULL REFERENCES human_accounts(id),
    request_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(created_by, request_id)
);
CREATE INDEX conversations_agent ON conversations(agent_id, id);
CREATE TABLE chat_turns (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    sequence bigint GENERATED ALWAYS AS IDENTITY UNIQUE,
    conversation_id uuid NOT NULL REFERENCES conversations(id),
    actor_id uuid NOT NULL REFERENCES human_accounts(id),
    request_id uuid NOT NULL,
    message text NOT NULL CHECK (octet_length(message) BETWEEN 1 AND 16384),
    status text NOT NULL DEFAULT 'queued' CHECK(status IN ('queued','running','waiting','complete')),
    response text CHECK(octet_length(response) <= 32768),
    error text,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    completed_at timestamptz,
    UNIQUE(actor_id, request_id),
    CHECK ((status='complete') = (response IS NOT NULL)),
    CHECK ((status='complete') = (completed_at IS NOT NULL))
);
CREATE INDEX chat_history ON chat_turns(conversation_id, sequence);
