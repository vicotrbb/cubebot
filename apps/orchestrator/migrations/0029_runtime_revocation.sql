-- An operator can revoke a sandbox incarnation's agent credential while the sandbox runs (ADR 0083).
-- A revoked session authorizes nothing, and its attempt cannot bootstrap a new credential.
ALTER TABLE agent_runtime_sessions ADD COLUMN revoked_at timestamptz;
CREATE OR REPLACE VIEW runtime_session_access AS
SELECT r.agent_id,r.id AS attempt_id,r.epoch,s.version,s.token_hash,
    (r.kind = 'pod' AND r.phase = 'bound' AND r.uid = s.pod_uid AND s.revoked_at IS NULL) AS recovery,
    (r.kind = 'pod' AND r.phase = 'bound' AND r.uid = s.pod_uid AND s.revoked_at IS NULL
        AND a.desired_state = 'running' AND a.restart_generation = r.target_generation
        AND r.blocked_reason IS NULL AND r.termination_proof IS NULL
        AND COALESCE(r.kubernetes_phase = 'Running',false) AND NOT r.deleting
        AND NOT EXISTS (SELECT 1 FROM agent_runtime_drains d WHERE d.attempt_id = r.id)) AS active
FROM agent_runtime_sessions s JOIN agent_resources r ON r.id = s.attempt_id JOIN agents a ON a.id = r.agent_id;
