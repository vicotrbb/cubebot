-- One connection, now either OpenAI or Anthropic. OpenAI cannot report its context window, so
-- it is configured; existing OpenAI connections get a conservative window that compacts early.
ALTER TABLE provider_connection
    ADD COLUMN provider text NOT NULL DEFAULT 'openai' CHECK (provider IN ('openai', 'anthropic')),
    ADD COLUMN context_window bigint CHECK (context_window BETWEEN 1024 AND 100000000);
UPDATE provider_connection SET context_window = 128000 WHERE context_window IS NULL;
ALTER TABLE provider_connection
    ADD CHECK (provider <> 'openai' OR context_window IS NOT NULL);
ALTER TABLE provider_commands
    ADD COLUMN provider text NOT NULL DEFAULT 'openai',
    ADD COLUMN context_window bigint;
UPDATE provider_commands SET context_window = 128000 WHERE context_window IS NULL;
