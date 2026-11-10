-- An agent asks to create an agent, room or team; a person approves or declines it in the console.
-- Approving runs the console's own creation as that person, so their authority applies. The
-- capability is on by default and switched off per agent.
ALTER TABLE agents ADD COLUMN creates boolean NOT NULL DEFAULT true;
ALTER TABLE agent_definition_revisions ADD COLUMN creates boolean NOT NULL DEFAULT true;

CREATE TABLE creation_requests (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id uuid NOT NULL REFERENCES agents(id),
    turn_id uuid NOT NULL REFERENCES chat_turns(id),
    -- The tool call, so a resent request records nothing new.
    call_id uuid NOT NULL UNIQUE,
    kind text NOT NULL CHECK (kind IN ('agent', 'room', 'team')),
    -- The tool input as the agent sent it; names resolve to ids only on approval, so a request
    -- may name an agent another pending request creates.
    input jsonb NOT NULL CHECK (octet_length(input::text) <= 131072),
    status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'declined', 'failed')),
    settled_by uuid REFERENCES human_accounts(id),
    settled_at timestamptz,
    -- Why it was declined or could not be created, as told to the agent.
    outcome text CHECK (octet_length(outcome) <= 2048),
    created_id uuid,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK ((status = 'pending') = (settled_at IS NULL))
);
CREATE INDEX creation_requests_pending ON creation_requests(agent_id) WHERE status = 'pending';
