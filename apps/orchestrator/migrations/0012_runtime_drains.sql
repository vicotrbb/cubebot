-- Quiescence is a trusted runtime report, never physical termination or journal acceptance.
CREATE TABLE agent_runtime_drains (
    attempt_id uuid PRIMARY KEY REFERENCES agent_resources(id),
    request_id uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    requested_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    acknowledged_at timestamptz,
    acknowledged_session_version bigint CHECK (acknowledged_session_version > 0),
    CHECK ((acknowledged_at IS NULL) = (acknowledged_session_version IS NULL))
);

INSERT INTO agent_runtime_drains (attempt_id)
SELECT r.id FROM agent_resources r JOIN agents a ON a.id = r.agent_id
WHERE r.kind = 'pod' AND r.phase IN ('creating','bound')
AND (a.desired_state <> 'running' OR a.restart_generation <> r.target_generation);
