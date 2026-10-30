-- Desktop sessions (ADRs 0039 and 0085): who watched an agent's desktop and when, and each time
-- they took or handed back control. Frames are never stored.
CREATE TABLE desktop_sessions (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id uuid NOT NULL REFERENCES agents(id),
    viewer_id uuid NOT NULL REFERENCES human_accounts(id),
    -- The arbiter's mode after each change the viewer made, in order: {"mode", "at"}, where
    -- "human" took control and any other mode handed it back.
    mode_changes jsonb NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(mode_changes) = 'array'),
    started_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    ended_at timestamptz CHECK (ended_at >= started_at)
);
CREATE INDEX desktop_sessions_agent ON desktop_sessions(agent_id, started_at);

-- The agent's first screenshot after each time a person handed control back: where it looked
-- at the screen again. The conversation's id is its agent's.
CREATE VIEW desktop_reobservations AS
SELECT DISTINCT ON (s.agent_id, s.id, c.ordinality)
    s.agent_id, r.turn_id, r.payload->>'id' AS call_id
FROM desktop_sessions s
CROSS JOIN LATERAL jsonb_array_elements(s.mode_changes) WITH ORDINALITY AS c(change, ordinality)
JOIN chat_turns t ON t.conversation_id = s.agent_id
JOIN turn_events r ON r.turn_id = t.id
    AND r.payload->>'kind' = 'tool_result'
    AND r.payload->'outcome'->>'status' = 'screenshot'
    AND r.created_at > (c.change->>'at')::timestamptz
WHERE c.change->>'mode' <> 'human'
ORDER BY s.agent_id, s.id, c.ordinality, r.created_at;
