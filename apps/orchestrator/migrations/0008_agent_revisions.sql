ALTER TABLE agents ADD COLUMN restart_generation bigint NOT NULL DEFAULT 0 CHECK (restart_generation >= 0);
ALTER TABLE agent_commands ADD COLUMN input_body jsonb;
CREATE TABLE agent_definition_revisions (
    agent_id uuid NOT NULL REFERENCES agents(id),
    revision bigint NOT NULL CHECK (revision > 0),
    instructions_revision bigint NOT NULL CHECK (instructions_revision > 0),
    name text NOT NULL,
    instructions text NOT NULL,
    idle_policy text NOT NULL CHECK (idle_policy IN ('sleep', 'keep_running')),
    actor_id uuid NOT NULL REFERENCES human_accounts(id),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (agent_id, revision)
);
INSERT INTO agent_definition_revisions (agent_id, revision, instructions_revision, name, instructions, idle_policy, actor_id)
SELECT id, revision, instructions_revision, name, instructions, idle_policy, created_by FROM agents;
