-- Agents are always on (docs/adr/agents.md#agent-idle-lifecycle): a new agent's sandbox runs from
-- creation, and "sleeping" is now the activity of an agent idle past its delay, while it dreams.
-- The stopped intent, which only a person sets, is renamed to say what it does.
ALTER TABLE agents DROP CONSTRAINT agents_desired_state_check;
UPDATE agents
SET desired_state = 'running', restart_generation = restart_generation + 1, revision = revision + 1
WHERE desired_state = 'sleeping';
ALTER TABLE agents
    ALTER COLUMN desired_state SET DEFAULT 'running',
    ADD CONSTRAINT agents_desired_state_check CHECK (desired_state IN ('stopped', 'running', 'retired'));
