-- Gemini and Grok join the providers.
ALTER TABLE provider_connection DROP CONSTRAINT provider_connection_provider_check;
ALTER TABLE provider_connection ADD CONSTRAINT provider_connection_provider_check
    CHECK (provider IN ('openai', 'anthropic', 'openrouter', 'gemini', 'grok'));
ALTER TABLE provider_saved DROP CONSTRAINT provider_saved_provider_check;
ALTER TABLE provider_saved ADD CONSTRAINT provider_saved_provider_check
    CHECK (provider IN ('openai', 'anthropic', 'openrouter', 'gemini', 'grok'));
