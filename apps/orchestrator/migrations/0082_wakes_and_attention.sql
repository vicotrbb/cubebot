CREATE TABLE wakes (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id uuid NOT NULL REFERENCES agents(id),
    instructions text NOT NULL CHECK (octet_length(instructions) BETWEEN 1 AND 4096),
    schedule text CHECK (octet_length(schedule) BETWEEN 1 AND 128),
    time_zone text NOT NULL DEFAULT 'UTC' CHECK (octet_length(time_zone) BETWEEN 1 AND 64),
    next_at timestamptz NOT NULL,
    set_by text NOT NULL CHECK (set_by IN ('agent', 'person')),
    created_by uuid NOT NULL REFERENCES human_accounts(id),
    request_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (agent_id, request_id)
);
CREATE INDEX wakes_agent ON wakes(agent_id, next_at);
CREATE INDEX wakes_due ON wakes(next_at);

ALTER TABLE chat_turns DROP CONSTRAINT chat_turns_origin_check;
ALTER TABLE chat_turns ADD CONSTRAINT chat_turns_origin_check
    CHECK (origin IN ('person', 'agent', 'schedule', 'webhook', 'event', 'wake'));
ALTER TABLE chat_turns
    ADD COLUMN wake_id uuid REFERENCES wakes(id) ON DELETE SET NULL,
    ADD COLUMN attention text CHECK (octet_length(attention) BETWEEN 1 AND 280),
    ADD COLUMN attention_settled_at timestamptz;
CREATE INDEX chat_turns_wake ON chat_turns(wake_id) WHERE wake_id IS NOT NULL;
CREATE INDEX chat_turns_attention ON chat_turns(conversation_id) WHERE attention IS NOT NULL AND attention_settled_at IS NULL;
