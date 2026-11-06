-- What a person tells agents about themselves (apps/orchestrator/src/identity/profiles.rs): a name,
-- pronouns, role, time zone, language and free text. Agents read it for the people who reach them.
ALTER TABLE human_accounts
    ADD COLUMN profile jsonb NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(profile) = 'object');
