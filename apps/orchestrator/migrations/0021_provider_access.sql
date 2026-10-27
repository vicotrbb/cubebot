ALTER TABLE provider_connection ADD COLUMN access jsonb NOT NULL DEFAULT '{"mode":"all_agents"}'::jsonb
    CHECK (access->>'mode' IN ('all_agents','explicit_grants'));
ALTER TABLE provider_commands ADD COLUMN access jsonb NOT NULL DEFAULT '{"mode":"all_agents"}'::jsonb;
