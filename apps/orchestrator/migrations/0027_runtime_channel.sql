-- One runtime channel per sandbox replaces per-turn grants, terminal leases and journal custody.
-- An agent credential lasts for its sandbox incarnation; revocation is the session or pod ending.
CREATE TABLE turn_events (
    turn_id uuid NOT NULL REFERENCES chat_turns(id),
    sequence bigint NOT NULL CHECK (sequence > 0),
    payload jsonb NOT NULL CHECK (payload->>'kind' IN ('text_delta','tool_call','tool_result','usage','done','error')),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (turn_id, sequence)
);

-- Keep earlier terminal history visible, including effects that never reported an outcome.
WITH numbered AS (
    SELECT i.*, row_number() OVER (PARTITION BY turn_id ORDER BY created_at, id) AS n
    FROM terminal_invocations i
)
INSERT INTO turn_events (turn_id, sequence, payload, created_at)
SELECT turn_id, 2 * n - 1,
    jsonb_build_object('kind','tool_call','id',id,'name','terminal','input',jsonb_build_object('command',request->>'command')),
    created_at
FROM numbered
UNION ALL
SELECT turn_id, 2 * n, jsonb_build_object('kind','tool_result','id',id,'outcome',outcome), created_at
FROM numbered WHERE outcome IS NOT NULL;

-- Running work held a grant that no longer exists; report it as interrupted, never as complete.
UPDATE chat_turns SET status='waiting', error='execution_interrupted' WHERE status IN ('running','retrying');
ALTER TABLE chat_turns DROP CONSTRAINT chat_turns_status_check;
ALTER TABLE chat_turns ADD CHECK (status IN ('queued','running','waiting','complete','cancelled'));
ALTER TABLE chat_turns ADD COLUMN attempt_id uuid REFERENCES agent_resources(id);
ALTER TABLE chat_turns DROP COLUMN authority_id;
ALTER TABLE chat_retries DROP COLUMN previous_authority;
CREATE UNIQUE INDEX chat_one_running_turn ON chat_turns (conversation_id) WHERE status = 'running';

DROP TABLE terminal_invocations;
DROP TABLE chat_authorities;

DROP VIEW runtime_session_access;
DROP TABLE agent_journal_records;
DROP TABLE agent_journal_streams;
ALTER TABLE agent_runtime_sessions DROP COLUMN expires_at;
CREATE VIEW runtime_session_access AS
SELECT r.agent_id,r.id AS attempt_id,r.epoch,s.version,s.token_hash,
    (r.kind = 'pod' AND r.phase = 'bound' AND r.uid = s.pod_uid) AS recovery,
    (r.kind = 'pod' AND r.phase = 'bound' AND r.uid = s.pod_uid
        AND a.desired_state = 'running' AND a.restart_generation = r.target_generation
        AND r.blocked_reason IS NULL AND r.termination_proof IS NULL
        AND COALESCE(r.kubernetes_phase = 'Running',false) AND NOT r.deleting
        AND NOT EXISTS (SELECT 1 FROM agent_runtime_drains d WHERE d.attempt_id = r.id)) AS active
FROM agent_runtime_sessions s JOIN agent_resources r ON r.id = s.attempt_id JOIN agents a ON a.id = r.agent_id;
