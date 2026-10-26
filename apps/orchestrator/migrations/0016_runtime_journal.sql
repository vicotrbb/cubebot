CREATE TABLE agent_journal_streams (
    agent_id uuid PRIMARY KEY REFERENCES agents(id),
    received_through bigint NOT NULL DEFAULT 0 CHECK (received_through >= 0)
);
CREATE TABLE agent_journal_records (
    agent_id uuid NOT NULL REFERENCES agent_journal_streams(agent_id),
    sequence bigint NOT NULL CHECK (sequence > 0),
    payload bytea NOT NULL CHECK (octet_length(payload) <= 65536),
    payload_digest bytea NOT NULL CHECK (octet_length(payload_digest) = 32),
    reported_attempt uuid NOT NULL REFERENCES agent_resources(id),
    received_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    reconciled_at timestamptz,
    PRIMARY KEY (agent_id,sequence)
);
-- Retention acknowledges custody, never semantic acceptance of an execution outcome.
CREATE OR REPLACE VIEW runtime_session_access AS
SELECT r.agent_id,r.id AS attempt_id,r.epoch,s.version,s.token_hash,s.expires_at,
    (r.kind = 'pod' AND r.phase = 'bound' AND r.uid = s.pod_uid AND s.expires_at > clock_timestamp()) AS recovery,
    (r.kind = 'pod' AND r.phase = 'bound' AND r.uid = s.pod_uid AND s.expires_at > clock_timestamp()
        AND a.desired_state = 'running' AND a.restart_generation = r.target_generation
        AND r.blocked_reason IS NULL AND r.termination_proof IS NULL
        AND COALESCE(r.kubernetes_phase = 'Running',false) AND NOT r.deleting
        AND NOT EXISTS (SELECT 1 FROM agent_runtime_drains d WHERE d.attempt_id = r.id)
        AND NOT EXISTS (SELECT 1 FROM agent_journal_records j WHERE j.agent_id = r.agent_id AND j.reconciled_at IS NULL)) AS active
FROM agent_runtime_sessions s JOIN agent_resources r ON r.id = s.attempt_id JOIN agents a ON a.id = r.agent_id;
CREATE INDEX agent_journal_unreconciled ON agent_journal_records (agent_id) WHERE reconciled_at IS NULL;
