CREATE TABLE identity_oidc_policy (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    enabled boolean NOT NULL DEFAULT false,
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    config_hash bytea
);
INSERT INTO identity_oidc_policy (singleton) VALUES (true);
ALTER TABLE human_sessions ADD COLUMN oidc_policy_revision bigint;
CREATE TABLE oidc_flows (
    state_hash bytea PRIMARY KEY CHECK (octet_length(state_hash) = 32),
    browser_hash bytea NOT NULL CHECK (octet_length(browser_hash) = 32),
    nonce text NOT NULL,
    policy_revision bigint NOT NULL,
    link_session_hash bytea REFERENCES human_sessions(token_hash) ON DELETE CASCADE,
    expires_at timestamptz NOT NULL DEFAULT clock_timestamp() + interval '10 minutes'
);
CREATE INDEX oidc_flows_expiry ON oidc_flows(expires_at);
CREATE TABLE oidc_bindings (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id uuid NOT NULL REFERENCES human_accounts(id),
    issuer text NOT NULL CHECK (octet_length(issuer) BETWEEN 1 AND 2048),
    subject text NOT NULL CHECK (octet_length(subject) BETWEEN 1 AND 255),
    active boolean NOT NULL DEFAULT true,
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    UNIQUE (issuer, subject)
);
CREATE UNIQUE INDEX oidc_binding_account ON oidc_bindings(account_id, issuer) WHERE active;
