-- Personal subscription connections (docs/adr/identity.md#personal-subscription-connections).
-- A ChatGPT or SuperGrok plan signed in by one person serves only that person's work. Its tokens
-- live here, encrypted and bound to the token revision; the connection row holds no key for it.
ALTER TABLE installation
    ADD COLUMN agent_host_id uuid NOT NULL DEFAULT gen_random_uuid(),
    ADD COLUMN chatgpt_client_id text CHECK (chatgpt_client_id ~ '^oaiapp_[A-Za-z0-9_-]{1,200}$');

ALTER TABLE provider_connection DROP CONSTRAINT provider_connection_provider_check;
ALTER TABLE provider_connection ADD CONSTRAINT provider_connection_provider_check
    CHECK (provider IN ('openai', 'anthropic', 'openrouter', 'gemini', 'grok', 'chatgpt', 'supergrok'));
ALTER TABLE provider_connection ALTER COLUMN credential DROP NOT NULL;
ALTER TABLE provider_connection ADD CONSTRAINT provider_connection_subscription_key
    CHECK ((provider IN ('chatgpt', 'supergrok')) = (credential IS NULL));

CREATE TABLE provider_subscriptions (
    provider text PRIMARY KEY CHECK (provider IN ('chatgpt', 'supergrok')),
    owner uuid NOT NULL REFERENCES human_accounts(id),
    account text NOT NULL CHECK (octet_length(account) BETWEEN 1 AND 320),
    credential bytea NOT NULL,
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    expires_at timestamptz NOT NULL,
    refresh_after timestamptz NOT NULL,
    -- sign_in_again: the vendor refused the refresh token. refresh_failing: retrying a transient failure.
    problem text CHECK (problem IN ('sign_in_again', 'refresh_failing')),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
