-- Members own what they create. `created_by` on agents, rooms, teams and connections is the
-- owner from here on: transferring ownership rewrites it, and the audit log keeps the history.

ALTER TABLE human_accounts DROP CONSTRAINT human_accounts_role_check;
ALTER TABLE human_accounts ADD CONSTRAINT human_accounts_role_check
    CHECK (role IN ('administrator', 'auditor', 'member', 'guest'));
ALTER TABLE human_accounts
    ADD COLUMN last_seen_at timestamptz,
    ADD COLUMN totp_secret bytea,
    ADD COLUMN totp_enabled_at timestamptz,
    -- The last accepted time step: a code works once.
    ADD COLUMN totp_last_step bigint,
    ADD CHECK (totp_enabled_at IS NULL OR totp_secret IS NOT NULL);

CREATE TABLE human_recovery_codes (
    account_id uuid NOT NULL REFERENCES human_accounts(id),
    code_hash bytea NOT NULL CHECK (octet_length(code_hash) = 32),
    used_at timestamptz,
    PRIMARY KEY (account_id, code_hash)
);

-- The installation owner is an administrator only it can demote; ownership moves by transfer.
ALTER TABLE installation ADD COLUMN owner_id uuid REFERENCES human_accounts(id);
UPDATE installation SET owner_id = (
    -- An active one: many installations suspend the bootstrap account once real ones exist.
    SELECT id FROM human_accounts WHERE role = 'administrator' ORDER BY active DESC, created_at, id LIMIT 1
);

ALTER TABLE installation_settings
    ADD COLUMN members_create_agents boolean NOT NULL DEFAULT true,
    ADD COLUMN members_create_rooms boolean NOT NULL DEFAULT true,
    ADD COLUMN members_create_teams boolean NOT NULL DEFAULT true,
    ADD COLUMN members_create_connections boolean NOT NULL DEFAULT true,
    ADD COLUMN members_bring_providers boolean NOT NULL DEFAULT true,
    ADD COLUMN member_agent_limit integer DEFAULT 3 CHECK (member_agent_limit IS NULL OR member_agent_limit BETWEEN 0 AND 10000);
-- An installation that already has people keeps what members could do before, nothing, until
-- an administrator allows more; a new one starts with members allowed.
UPDATE installation_settings SET members_create_agents = false, members_create_rooms = false,
    members_create_teams = false, members_create_connections = false,
    members_bring_providers = false
WHERE EXISTS (SELECT 1 FROM human_accounts);

ALTER TABLE invitations ADD COLUMN role text NOT NULL DEFAULT 'member'
    CHECK (role IN ('auditor', 'member', 'guest'));

-- A session row is a browser sign-in, or a personal API token when it has a name.
ALTER TABLE human_sessions
    ADD COLUMN id uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    ADD COLUMN name text CHECK (name IS NULL OR length(btrim(name)) BETWEEN 1 AND 100),
    ADD COLUMN last_seen_at timestamptz;

CREATE TABLE password_resets (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id uuid NOT NULL REFERENCES human_accounts(id),
    token_hash bytea NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    created_by uuid NOT NULL REFERENCES human_accounts(id),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    expires_at timestamptz NOT NULL DEFAULT clock_timestamp() + interval '1 day',
    used_at timestamptz
);
-- A link works only while its maker could still make it: an active administrator, and the owner
-- if the account it resets has since become an administrator.
CREATE FUNCTION reset_still_authorized(account uuid, maker uuid) RETURNS boolean
LANGUAGE sql STABLE AS $$
    SELECT EXISTS(SELECT 1 FROM human_accounts WHERE id = $2 AND active AND role = 'administrator')
       AND ((SELECT role <> 'administrator' FROM human_accounts WHERE id = $1)
            OR $2 = (SELECT owner_id FROM installation WHERE singleton))
$$;

-- Owners hold every capability on what they own while they may own things (a guest or auditor
-- holds none on what it once created), and auditors read everything, as grants the
-- view derives; grant rows stay the explicit shares. `source` says where each row comes from.
CREATE OR REPLACE VIEW effective_human_grants AS
SELECT a.id AS actor_id, g.id, g.scope_kind, g.scope_id, g.capability, g.team_id, 'grant'::text AS source
FROM human_accounts a JOIN access_grants g ON g.account_id = a.id
WHERE a.active AND g.revoked_at IS NULL
UNION ALL
SELECT a.id, g.id, g.scope_kind, g.scope_id, g.capability, g.team_id, 'group'
FROM human_accounts a
JOIN human_team_members m ON m.account_id = a.id
JOIN human_teams t ON t.id = m.team_id AND t.active
JOIN access_grants g ON g.team_id = t.id
WHERE a.active AND g.revoked_at IS NULL
UNION ALL
SELECT a.id, NULL::uuid, c.kind, g.id, c.capability, NULL::uuid, 'owner'
FROM human_accounts a
JOIN agents g ON g.created_by = a.id
CROSS JOIN (VALUES
    ('agent', 'agent.manage'), ('agent', 'memory.read'), ('agent', 'memory.edit'),
    ('agent', 'workspace.read'), ('agent', 'workspace.write'), ('agent', 'browser.view'),
    ('agent', 'browser.control'), ('conversation', 'conversation.read'),
    ('conversation', 'conversation.send')
) c(kind, capability)
WHERE a.active AND a.role IN ('administrator', 'member')
UNION ALL
SELECT a.id, NULL::uuid, 'room', r.id, c.capability, NULL::uuid, 'owner'
FROM human_accounts a
JOIN rooms r ON r.created_by = a.id AND NOT r.task
CROSS JOIN (VALUES ('conversation.read'), ('conversation.send'), ('room.manage')) c(capability)
WHERE a.active AND a.role IN ('administrator', 'member')
UNION ALL
SELECT a.id, NULL::uuid, c.kind, g.id, c.capability, NULL::uuid, 'auditor'
FROM human_accounts a
JOIN agents g ON a.role = 'auditor'
CROSS JOIN (VALUES
    ('agent', 'memory.read'), ('agent', 'workspace.read'), ('agent', 'browser.view'),
    ('conversation', 'conversation.read')
) c(kind, capability)
WHERE a.active
UNION ALL
SELECT a.id, NULL::uuid, 'room', r.id, 'conversation.read', NULL::uuid, 'auditor'
FROM human_accounts a
JOIN rooms r ON a.role = 'auditor' AND NOT r.task
WHERE a.active;
