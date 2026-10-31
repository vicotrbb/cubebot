-- Rooms, agent-to-agent work and steering (ADRs 0014-0016, 0071-0073). Every delivery stays a
-- turn in its recipient's permanent conversation; rooms and ancestry only say where it came from.
CREATE TABLE rooms (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL CHECK (octet_length(name) BETWEEN 1 AND 128),
    created_by uuid NOT NULL REFERENCES human_accounts(id),
    request_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(created_by, request_id)
);
CREATE TABLE room_members (
    room_id uuid NOT NULL REFERENCES rooms(id),
    agent_id uuid NOT NULL REFERENCES agents(id),
    PRIMARY KEY (room_id, agent_id)
);
CREATE INDEX room_members_agent ON room_members(agent_id);
CREATE TABLE room_messages (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    sequence bigint GENERATED ALWAYS AS IDENTITY UNIQUE,
    room_id uuid NOT NULL REFERENCES rooms(id),
    actor_id uuid NOT NULL REFERENCES human_accounts(id),
    request_id uuid NOT NULL,
    message text NOT NULL CHECK (octet_length(message) BETWEEN 1 AND 16384),
    -- routing: waiting for the coordinator; the rest are its settled results. continued: added
    -- as steering to an open task of this room, named in note.
    routing text NOT NULL CHECK (routing IN ('routing','routed','continued','clarify','no_action','failed')),
    note text CHECK (octet_length(note) <= 4096),
    attempts smallint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE(actor_id, request_id)
);
CREATE INDEX room_messages_history ON room_messages(room_id, sequence);
CREATE INDEX room_messages_routing ON room_messages(sequence) WHERE routing = 'routing';

ALTER TABLE chat_turns
    ADD COLUMN room_message_id uuid REFERENCES room_messages(id),
    ADD COLUMN parent_turn_id uuid REFERENCES chat_turns(id),
    -- The parent yields until this child settles (delegate), or not (send_message).
    ADD COLUMN awaited boolean NOT NULL DEFAULT false,
    -- The child had settled when a run of its parent started, so that run saw its outcome.
    ADD COLUMN seen boolean NOT NULL DEFAULT false,
    -- A send_message whose sender wants the reply relayed back once it completes.
    ADD COLUMN expects_reply boolean NOT NULL DEFAULT false,
    ADD CHECK (NOT awaited OR parent_turn_id IS NOT NULL);
-- Room and agent deliveries carry their provenance ahead of the 16 KiB message.
ALTER TABLE chat_turns DROP CONSTRAINT chat_turns_message_check;
ALTER TABLE chat_turns ADD CHECK (octet_length(message) BETWEEN 1 AND 20480);
CREATE INDEX chat_turns_children ON chat_turns(parent_turn_id) WHERE parent_turn_id IS NOT NULL;
CREATE UNIQUE INDEX chat_turns_room_delivery ON chat_turns(room_message_id, conversation_id)
    WHERE room_message_id IS NOT NULL;
-- delegated: yielded while awaited children run. failed: agent-originated work that ended in an
-- error settles, so its parent receives the evidence instead of waiting for a person.
ALTER TABLE chat_turns DROP CONSTRAINT chat_turns_status_check;
ALTER TABLE chat_turns ADD CHECK (status IN ('queued','running','waiting','delegated','complete','failed','cancelled'));

CREATE TABLE turn_steering (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    turn_id uuid NOT NULL REFERENCES chat_turns(id),
    actor_id uuid NOT NULL REFERENCES human_accounts(id),
    request_id uuid NOT NULL,
    message text NOT NULL CHECK (octet_length(message) BETWEEN 1 AND 16384),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    incorporated_at timestamptz,
    -- Copied to an open descendant from this steering of an ancestor.
    source_steering_id uuid REFERENCES turn_steering(id),
    UNIQUE(actor_id, request_id),
    UNIQUE(source_steering_id, turn_id)
);
CREATE INDEX turn_steering_turn ON turn_steering(turn_id, created_at);

ALTER TABLE turn_events DROP CONSTRAINT turn_events_payload_check;
ALTER TABLE turn_events ADD CHECK (payload->>'kind' IN ('text_delta','tool_call','tool_result','usage','summary','steered','done','error'));

-- Lanes (ADR 0005): direct chat, room work and messages share one main lane; each delegated
-- task runs in a task lane beside it, up to the agent's task_lanes.
ALTER TABLE agents ADD COLUMN task_lanes smallint NOT NULL DEFAULT 2 CHECK (task_lanes BETWEEN 1 AND 4);
ALTER TABLE agent_definition_revisions ADD COLUMN task_lanes smallint NOT NULL DEFAULT 2;
DROP INDEX chat_one_running_turn;
CREATE UNIQUE INDEX chat_one_running_main_turn ON chat_turns (conversation_id) WHERE status = 'running' AND NOT awaited;
