-- Settings administrators change in the console; NULL keeps the deployment's value.
CREATE TABLE installation_settings (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    name text CHECK (name IS NULL OR char_length(name) BETWEEN 1 AND 64),
    storage_quota_bytes bigint CHECK (storage_quota_bytes IS NULL OR storage_quota_bytes > 0),
    upload_limit_bytes bigint CHECK (upload_limit_bytes IS NULL OR upload_limit_bytes > 0),
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0)
);
INSERT INTO installation_settings (singleton) VALUES (true);

-- Single sign-on set up in the console: the provider settings, and the client secret encrypted
-- with the installation's credential key.
ALTER TABLE identity_oidc_policy ADD COLUMN config jsonb, ADD COLUMN client_secret bytea;
