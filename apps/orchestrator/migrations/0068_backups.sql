-- Backups to an S3-compatible bucket (docs/adr/packaging.md#backups-without-pauses). Settings set
-- by the deployment (Helm values or .env) win over these; NULL means unset here.
CREATE TABLE backup_settings (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    endpoint text,
    region text,
    bucket text,
    access_key_id text,
    -- Encrypted with the installation's credential key.
    secret_access_key bytea,
    schedule text CHECK (schedule IN ('daily', 'off')),
    keep_daily integer CHECK (keep_daily BETWEEN 1 AND 365),
    keep_weekly integer CHECK (keep_weekly BETWEEN 0 AND 520),
    bandwidth_bytes bigint CHECK (bandwidth_bytes >= 1048576),
    concurrency integer CHECK (concurrency BETWEEN 1 AND 16),
    -- The credential key may be downloaded from the console once.
    key_downloaded_at timestamptz,
    key_downloaded_by uuid REFERENCES human_accounts(id),
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0)
);
INSERT INTO backup_settings (singleton) VALUES (true);

-- One row per backup; the id, a UTC time such as 20261107T030000Z, is its prefix in the bucket.
CREATE TABLE backups (
    id text PRIMARY KEY CHECK (id ~ '^[0-9]{8}T[0-9]{6}Z$'),
    requested_by uuid REFERENCES human_accounts(id),
    started_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    finished_at timestamptz,
    -- complete: verified with every part; incomplete: verified, with the parts in missing absent.
    status text NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'complete', 'incomplete', 'failed', 'deleted')),
    bytes bigint NOT NULL DEFAULT 0,
    missing jsonb NOT NULL DEFAULT '[]',
    error text
);

-- Each agent's workspace, which the agent streams through the orchestrator.
CREATE TABLE backup_workspaces (
    backup_id text NOT NULL REFERENCES backups(id) ON DELETE CASCADE,
    agent_id uuid NOT NULL REFERENCES agents(id),
    status text NOT NULL DEFAULT 'requested' CHECK (status IN ('requested', 'uploading', 'done', 'missing')),
    bytes bigint,
    sha256 bytea,
    error text,
    PRIMARY KEY (backup_id, agent_id)
);

-- Workspaces a restore still has to put back; each agent fetches its own before it takes work.
CREATE TABLE workspace_restores (
    agent_id uuid PRIMARY KEY REFERENCES agents(id),
    backup_id text NOT NULL,
    bytes bigint NOT NULL,
    sha256 bytea NOT NULL CHECK (octet_length(sha256) = 32),
    restored_at timestamptz,
    error text
);
