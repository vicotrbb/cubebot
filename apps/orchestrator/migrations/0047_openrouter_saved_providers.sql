-- OpenRouter joins the providers. Agents still think with one connection, but a provider that
-- stops being it keeps its key here, encrypted, so switching back needs no pasting.
ALTER TABLE provider_connection DROP CONSTRAINT provider_connection_provider_check;
ALTER TABLE provider_connection ADD CONSTRAINT provider_connection_provider_check
    CHECK (provider IN ('openai', 'anthropic', 'openrouter'));

CREATE TABLE provider_saved (
    provider text PRIMARY KEY CHECK (provider IN ('openai', 'anthropic', 'openrouter')),
    model text NOT NULL,
    context_window bigint CHECK (context_window BETWEEN 1024 AND 100000000),
    credential bytea NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
