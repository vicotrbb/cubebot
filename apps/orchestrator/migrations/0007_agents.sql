-- Identity owns the effective grant relation used by domain list/read queries.
CREATE VIEW effective_human_grants AS
SELECT a.id AS actor_id, g.id, g.scope_kind, g.scope_id, g.capability, g.team_id
FROM human_accounts a JOIN access_grants g ON g.account_id = a.id
WHERE a.active AND g.revoked_at IS NULL
UNION ALL
SELECT a.id AS actor_id, g.id, g.scope_kind, g.scope_id, g.capability, g.team_id
FROM human_accounts a
JOIN human_team_members m ON m.account_id = a.id
JOIN human_teams t ON t.id = m.team_id AND t.active
JOIN access_grants g ON g.team_id = t.id
WHERE a.active AND g.revoked_at IS NULL;

CREATE TABLE agents (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL CHECK (octet_length(name) BETWEEN 1 AND 255),
    instructions text NOT NULL CHECK (octet_length(instructions) <= 65536),
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    instructions_revision bigint NOT NULL DEFAULT 1 CHECK (instructions_revision > 0),
    desired_state text NOT NULL DEFAULT 'sleeping' CHECK (desired_state IN ('sleeping', 'running', 'retired')),
    idle_policy text NOT NULL DEFAULT 'sleep' CHECK (idle_policy IN ('sleep', 'keep_running')),
    created_by uuid NOT NULL REFERENCES human_accounts(id),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE agent_commands (
    actor_id uuid NOT NULL REFERENCES human_accounts(id),
    kind text NOT NULL,
    request_id uuid NOT NULL CHECK (request_id <> '00000000-0000-0000-0000-000000000000'),
    input_digest bytea NOT NULL CHECK (octet_length(input_digest) = 32),
    agent_id uuid NOT NULL REFERENCES agents(id),
    revision bigint NOT NULL CHECK (revision > 0),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (actor_id, kind, request_id)
);
CREATE TABLE agent_events (
    sequence bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    actor_id uuid NOT NULL,
    kind text NOT NULL,
    request_id uuid NOT NULL,
    agent_id uuid NOT NULL REFERENCES agents(id),
    revision bigint NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    FOREIGN KEY (actor_id, kind, request_id) REFERENCES agent_commands(actor_id, kind, request_id)
);
