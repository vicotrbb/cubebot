-- Idle dreaming (ADR 0025): how long an idle agent waits before it dreams, and the model it
-- dreams with on the agent's connection (ADR 0029); NULL inherits the connection's model.
ALTER TABLE agents
    ADD COLUMN idle_delay_seconds integer NOT NULL DEFAULT 600 CHECK (idle_delay_seconds BETWEEN 60 AND 86400),
    ADD COLUMN dreaming_model text CHECK (char_length(dreaming_model) BETWEEN 1 AND 200);
ALTER TABLE agent_definition_revisions
    ADD COLUMN idle_delay_seconds integer NOT NULL DEFAULT 600,
    ADD COLUMN dreaming_model text;

-- One row per admitted run. The outcome is set when it ends; a completed run that committed no
-- revision is recorded as no_op.
CREATE TABLE dream_runs (
    id uuid PRIMARY KEY,
    agent_id uuid NOT NULL REFERENCES agents(id),
    attempt_id uuid NOT NULL REFERENCES agent_resources(id),
    model text NOT NULL,
    status text NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'finished')),
    started_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    ended_at timestamptz,
    outcome text CHECK (outcome IN ('completed', 'yielded', 'failed', 'no_op')),
    report text NOT NULL DEFAULT '' CHECK (octet_length(report) <= 8192),
    CHECK ((status = 'running') = (outcome IS NULL) AND (outcome IS NULL) = (ended_at IS NULL))
);
CREATE INDEX dream_runs_agent ON dream_runs(agent_id, started_at DESC);

ALTER TABLE memory_revisions ADD COLUMN dream_run_id uuid REFERENCES dream_runs(id);
ALTER TABLE memory_revisions DROP CONSTRAINT memory_revisions_check;
ALTER TABLE memory_revisions ADD CHECK (num_nonnulls(human_id, turn_id, dream_run_id) = 1);
CREATE INDEX memory_revisions_dream ON memory_revisions(dream_run_id) WHERE dream_run_id IS NOT NULL;
