-- One explicitly selected API-key provider/model for the text-chat MVP.
CREATE TABLE provider_connection (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    model text NOT NULL CHECK (octet_length(model) BETWEEN 1 AND 128),
    credential bytea NOT NULL CHECK (octet_length(credential) >= 29),
    key_version bigint NOT NULL CHECK (key_version = 1),
    revision bigint NOT NULL CHECK (revision > 0),
    updated_by uuid NOT NULL REFERENCES human_accounts(id),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE TABLE provider_commands (
    actor_id uuid NOT NULL REFERENCES human_accounts(id),
    request_id uuid NOT NULL,
    input_digest bytea NOT NULL CHECK (octet_length(input_digest) = 32),
    model text NOT NULL,
    revision bigint NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (actor_id, request_id)
);
