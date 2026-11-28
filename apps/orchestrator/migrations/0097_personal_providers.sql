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

-- The keys that may serve right now: their owner is active, an administrator, or a member while
-- members may bring their own.
CREATE VIEW usable_personal_providers AS
SELECT p.*
FROM personal_providers p
JOIN human_accounts h ON h.id = p.owner AND h.active
WHERE h.role = 'administrator'
   OR (h.role = 'member' AND (SELECT members_bring_providers FROM installation_settings WHERE singleton));
