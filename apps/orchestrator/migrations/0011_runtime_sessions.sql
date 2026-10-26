CREATE TABLE agent_runtime_sessions (
    attempt_id uuid PRIMARY KEY REFERENCES agent_resources(id),
    instance_id uuid NOT NULL CHECK (instance_id <> '00000000-0000-0000-0000-000000000000'),
    pod_uid text NOT NULL CHECK (octet_length(pod_uid) BETWEEN 1 AND 255),
    service_account_uid text NOT NULL CHECK (octet_length(service_account_uid) BETWEEN 1 AND 255),
    token_hash bytea NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    version bigint NOT NULL DEFAULT 1 CHECK (version > 0),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    expires_at timestamptz NOT NULL CHECK (expires_at > created_at)
);
