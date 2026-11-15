ALTER TABLE provider_connection ADD COLUMN reasoning_effort text
    CHECK (reasoning_effort IN ('minimal', 'low', 'medium', 'high', 'xhigh', 'max'));
ALTER TABLE provider_saved ADD COLUMN reasoning_effort text
    CHECK (reasoning_effort IN ('minimal', 'low', 'medium', 'high', 'xhigh', 'max'));
ALTER TABLE provider_commands ADD COLUMN reasoning_effort text;
ALTER TABLE agents ADD COLUMN model_effort text
    CHECK (model_effort IN ('minimal', 'low', 'medium', 'high', 'xhigh', 'max'));
