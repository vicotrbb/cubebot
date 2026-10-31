-- Login handoffs (ADR 0039): an agent's request that a person log in on a site through its
-- desktop, who took control for it, and how it settled. At most one is pending per agent.
CREATE TABLE login_handoffs (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id uuid NOT NULL REFERENCES agents(id),
    turn_id uuid NOT NULL REFERENCES chat_turns(id),
    -- The request_login tool call, so a resent request records nothing new.
    call_id uuid NOT NULL UNIQUE,
    site text NOT NULL CHECK (octet_length(site) BETWEEN 1 AND 255),
    reason text NOT NULL CHECK (octet_length(reason) BETWEEN 1 AND 2048),
    status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'completed', 'declined')),
    -- The latest person to take control while it was pending.
    taken_by uuid REFERENCES human_accounts(id),
    taken_at timestamptz,
    settled_by uuid REFERENCES human_accounts(id),
    settled_at timestamptz,
    declined_because text CHECK (octet_length(declined_because) <= 2048),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK ((status = 'pending') = (settled_at IS NULL)),
    CHECK ((status = 'declined') = (declined_because IS NOT NULL))
);
CREATE UNIQUE INDEX login_handoffs_one_pending ON login_handoffs(agent_id) WHERE status = 'pending';
