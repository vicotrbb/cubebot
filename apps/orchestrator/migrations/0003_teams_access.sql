CREATE TABLE human_teams (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL CHECK (length(btrim(name)) BETWEEN 1 AND 100),
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    active boolean NOT NULL DEFAULT true
);
CREATE UNIQUE INDEX human_teams_active_name ON human_teams(name) WHERE active;
CREATE TABLE human_team_members (
    team_id uuid NOT NULL REFERENCES human_teams(id),
    account_id uuid NOT NULL REFERENCES human_accounts(id),
    PRIMARY KEY (team_id, account_id)
);
CREATE INDEX human_team_member_accounts ON human_team_members(account_id);

CREATE TABLE access_grants (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id uuid REFERENCES human_accounts(id),
    team_id uuid REFERENCES human_teams(id),
    scope_kind text NOT NULL CHECK (scope_kind IN ('agent', 'room', 'conversation')),
    scope_id uuid NOT NULL,
    capability text NOT NULL CHECK (capability IN (
        'agent.manage', 'conversation.read', 'conversation.send', 'memory.read',
        'memory.edit', 'workspace.read', 'workspace.write', 'browser.view',
        'browser.control', 'room.manage'
    )),
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    revoked_at timestamptz,
    CHECK (num_nonnulls(account_id, team_id) = 1),
    CHECK (scope_id <> '00000000-0000-0000-0000-000000000000'),
    CHECK (
        (capability IN ('agent.manage', 'memory.read', 'memory.edit', 'workspace.read', 'workspace.write', 'browser.view', 'browser.control') AND scope_kind = 'agent')
        OR (capability IN ('conversation.read', 'conversation.send') AND scope_kind IN ('room', 'conversation'))
        OR (capability = 'room.manage' AND scope_kind = 'room')
    )
);
CREATE UNIQUE INDEX access_grants_current ON access_grants
    (account_id, team_id, scope_kind, scope_id, capability) NULLS NOT DISTINCT
    WHERE revoked_at IS NULL;
CREATE INDEX access_grants_lookup ON access_grants(scope_kind, scope_id, capability)
    WHERE revoked_at IS NULL;

-- Durable command identities also preserve the actor and result revision for audit.
-- Digests here cover non-secret team/grant requests, never passwords or bearer tokens.
CREATE TABLE identity_commands (
    actor_id uuid NOT NULL REFERENCES human_accounts(id),
    kind text NOT NULL,
    request_id uuid NOT NULL CHECK (request_id <> '00000000-0000-0000-0000-000000000000'),
    input_body jsonb NOT NULL,
    input_digest bytea NOT NULL CHECK (octet_length(input_digest) = 32),
    outcome_id uuid NOT NULL,
    outcome_revision bigint NOT NULL CHECK (outcome_revision > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (actor_id, kind, request_id)
);
CREATE TABLE identity_events (
    sequence bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    actor_id uuid NOT NULL REFERENCES human_accounts(id),
    kind text NOT NULL,
    request_id uuid NOT NULL,
    resource_id uuid NOT NULL,
    revision bigint NOT NULL CHECK (revision > 0),
    created_at timestamptz NOT NULL DEFAULT now(),
    FOREIGN KEY (actor_id, kind, request_id) REFERENCES identity_commands(actor_id, kind, request_id)
);
