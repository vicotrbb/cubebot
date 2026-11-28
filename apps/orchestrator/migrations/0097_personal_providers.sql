-- A person's own API key for a provider; their agents that choose that provider use it, before
-- any installation key.
CREATE TABLE personal_providers (
    owner uuid NOT NULL REFERENCES human_accounts(id),
    provider text NOT NULL CHECK (provider IN ('openai', 'anthropic', 'openrouter', 'gemini', 'grok', 'minimax', 'minimax_cn')),
    credential bytea NOT NULL,
    context_window bigint CHECK (context_window BETWEEN 1024 AND 100000000),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (owner, provider)
);
