-- Vault items (ADR 0086): logins and secrets an agent fills into its desktop without reading
-- them. An installation item has no agent and an ADR 0081 access policy; an agent item belongs
-- to that agent alone. The secret is a Credentials record bound to the item's id.
CREATE TABLE vault_items (
    id uuid PRIMARY KEY,
    agent_id uuid REFERENCES agents(id),
    kind text NOT NULL CHECK (kind IN ('login', 'secret')),
    name text NOT NULL CHECK (octet_length(name) BETWEEN 1 AND 255),
    site text CHECK (octet_length(site) BETWEEN 1 AND 255),
    username text CHECK (octet_length(username) BETWEEN 1 AND 255),
    ciphertext bytea NOT NULL,
    access jsonb NOT NULL DEFAULT '{"mode":"all_agents"}',
    -- NULL when the item's agent saved it with vault_put.
    created_by uuid REFERENCES human_accounts(id),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    last_used_at timestamptz,
    CHECK ((kind = 'login') = (site IS NOT NULL AND username IS NOT NULL)),
    CHECK (kind = 'login' OR (site IS NULL AND username IS NULL)),
    CHECK (created_by IS NOT NULL OR agent_id IS NOT NULL),
    CHECK (agent_id IS NULL OR access = '{"mode":"all_agents"}'),
    UNIQUE NULLS NOT DISTINCT (agent_id, name)
);
