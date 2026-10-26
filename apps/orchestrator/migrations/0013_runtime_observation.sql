ALTER TABLE agent_resources
    ADD COLUMN observed_at timestamptz,
    ADD COLUMN kubernetes_phase text CHECK (kubernetes_phase IN ('Pending','Running','Succeeded','Failed','Unknown')),
    ADD COLUMN kubernetes_reason text CHECK (kubernetes_reason ~ '^[A-Za-z0-9]{1,128}$'),
    ADD COLUMN kubernetes_ready boolean NOT NULL DEFAULT false,
    ADD COLUMN deleting boolean NOT NULL DEFAULT false;

-- One current policy for machine authentication, control and human status reporting.
-- Recovery identity permits reporting only; active eligibility is not per-turn authority.
CREATE VIEW runtime_session_access AS
SELECT r.agent_id,r.id AS attempt_id,r.epoch,s.version,s.token_hash,s.expires_at,
    (r.kind = 'pod' AND r.phase = 'bound' AND r.uid = s.pod_uid AND s.expires_at > clock_timestamp()) AS recovery,
    (r.kind = 'pod' AND r.phase = 'bound' AND r.uid = s.pod_uid AND s.expires_at > clock_timestamp()
        AND a.desired_state = 'running' AND a.restart_generation = r.target_generation
        AND r.blocked_reason IS NULL AND r.termination_proof IS NULL
        AND COALESCE(r.kubernetes_phase = 'Running',false) AND NOT r.deleting
        AND NOT EXISTS (SELECT 1 FROM agent_runtime_drains d WHERE d.attempt_id = r.id)) AS active
FROM agent_runtime_sessions s JOIN agent_resources r ON r.id = s.attempt_id JOIN agents a ON a.id = r.agent_id;
