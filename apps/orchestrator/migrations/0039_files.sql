-- Shared files (ADRs 0059 and 0060). A blob is immutable bytes on the storage volume, linked
-- into place and synchronized before its row commits; a row never names missing bytes. A file
-- is a folder (no blob) or one blob in a tree owned by one person or one agent; children share
-- their parent's owner. Rows are removed only by an explicit delete (ADR 0079).
CREATE TABLE blobs (
    id uuid PRIMARY KEY,
    size bigint NOT NULL CHECK (size >= 0),
    sha256 bytea NOT NULL CHECK (octet_length(sha256) = 32),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE files (
    id uuid PRIMARY KEY,
    parent_id uuid REFERENCES files(id),
    owner_account_id uuid REFERENCES human_accounts(id),
    owner_agent_id uuid REFERENCES agents(id),
    name text NOT NULL CHECK (octet_length(name) BETWEEN 1 AND 255 AND name NOT IN ('.', '..')
        AND strpos(name, '/') = 0),
    blob_id uuid UNIQUE REFERENCES blobs(id),
    content_type text,
    -- The uploader's request id, so a retried upload returns the file it already made.
    request_id uuid UNIQUE,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (num_nonnulls(owner_account_id, owner_agent_id) = 1),
    CHECK (blob_id IS NOT NULL OR content_type IS NULL),
    CONSTRAINT files_name_key UNIQUE NULLS NOT DISTINCT (owner_account_id, owner_agent_id, parent_id, name)
);
CREATE INDEX files_parent ON files(parent_id);
-- A share applies to the file and, for a folder, everything under it.
CREATE TABLE file_shares (
    file_id uuid NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    account_id uuid REFERENCES human_accounts(id),
    team_id uuid REFERENCES human_teams(id),
    agent_id uuid REFERENCES agents(id),
    permission text NOT NULL CHECK (permission IN ('read', 'write')),
    CHECK (num_nonnulls(account_id, team_id, agent_id) = 1),
    UNIQUE NULLS NOT DISTINCT (file_id, account_id, team_id, agent_id)
);
CREATE INDEX file_shares_account ON file_shares(account_id);
CREATE INDEX file_shares_team ON file_shares(team_id);
CREATE INDEX file_shares_agent ON file_shares(agent_id);
