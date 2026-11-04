-- Broadcasts: one message to every agent. People with the administrator role send them from
-- the console; an agent sends them only with its broadcast capability on. Each delivery is a
-- turn in the recipient's own conversation, unique per broadcast and agent, as room deliveries.
ALTER TABLE agents ADD COLUMN broadcast boolean NOT NULL DEFAULT false;
ALTER TABLE agent_definition_revisions ADD COLUMN broadcast boolean NOT NULL DEFAULT false;

CREATE TABLE broadcasts (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    sequence bigint GENERATED ALWAYS AS IDENTITY UNIQUE,
    -- The person whose authority the deliveries carry: the sender, or the person whose request
    -- the broadcasting agent's turn serves.
    actor_id uuid NOT NULL REFERENCES human_accounts(id),
    -- The agent that broadcast it; NULL when a person did.
    agent_id uuid REFERENCES agents(id),
    -- The person's request id, or the agent's tool call id.
    request_id uuid NOT NULL UNIQUE,
    -- At most one broadcast per agent turn.
    turn_id uuid UNIQUE REFERENCES chat_turns(id),
    message text NOT NULL CHECK (octet_length(message) BETWEEN 1 AND 16384),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK ((agent_id IS NULL) = (turn_id IS NULL))
);

ALTER TABLE chat_turns ADD COLUMN broadcast_id uuid REFERENCES broadcasts(id);
CREATE UNIQUE INDEX chat_turns_broadcast_delivery ON chat_turns (broadcast_id, conversation_id)
    WHERE broadcast_id IS NOT NULL;

ALTER TABLE read_cursors DROP CONSTRAINT read_cursors_scope_kind_check;
ALTER TABLE read_cursors ADD CONSTRAINT read_cursors_scope_kind_check
    CHECK (scope_kind IN ('conversation', 'room', 'broadcast'));
