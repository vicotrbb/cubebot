-- MiniMax joins the providers, on its global (`minimax`) or mainland China (`minimax_cn`) platform.
-- An M Plan key is personal like a signed-in plan, so it lives in provider_subscriptions and the
-- connection row holds no key for it.
ALTER TABLE provider_connection DROP CONSTRAINT provider_connection_provider_check;
ALTER TABLE provider_connection ADD CONSTRAINT provider_connection_provider_check
    CHECK (provider IN ('openai', 'anthropic', 'openrouter', 'gemini', 'grok', 'chatgpt', 'supergrok',
        'minimax', 'minimax_cn', 'minimax_plan', 'minimax_plan_cn'));
ALTER TABLE provider_connection DROP CONSTRAINT provider_connection_subscription_key;
ALTER TABLE provider_connection ADD CONSTRAINT provider_connection_subscription_key
    CHECK ((provider IN ('chatgpt', 'supergrok', 'minimax_plan', 'minimax_plan_cn')) = (credential IS NULL));

ALTER TABLE provider_saved DROP CONSTRAINT provider_saved_provider_check;
ALTER TABLE provider_saved ADD CONSTRAINT provider_saved_provider_check
    CHECK (provider IN ('openai', 'anthropic', 'openrouter', 'gemini', 'grok', 'minimax', 'minimax_cn'));

ALTER TABLE provider_subscriptions DROP CONSTRAINT provider_subscriptions_provider_check;
ALTER TABLE provider_subscriptions ADD CONSTRAINT provider_subscriptions_provider_check
    CHECK (provider IN ('chatgpt', 'supergrok', 'minimax_plan', 'minimax_plan_cn'));

ALTER TABLE task_settings DROP CONSTRAINT task_settings_provider_check;
ALTER TABLE task_settings ADD CONSTRAINT task_settings_provider_check
    CHECK (provider IN ('openai', 'anthropic', 'openrouter', 'gemini', 'grok', 'minimax', 'minimax_cn'));
