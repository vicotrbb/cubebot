-- A task's turn runs in a lane of its own beside the main lane, counted with the agent's other task
-- lanes (docs/adr/agents.md#agent-execution-lanes), so one task never waits for another task or for chat.
ALTER TABLE chat_turns ADD COLUMN in_task boolean NOT NULL DEFAULT false;
UPDATE chat_turns c
SET in_task = true
FROM room_messages m
JOIN rooms r ON r.id = m.room_id
WHERE m.id = c.room_message_id AND r.task;

DROP INDEX chat_one_running_main_turn;
CREATE UNIQUE INDEX chat_one_running_main_turn ON chat_turns (conversation_id)
    WHERE status = 'running' AND NOT awaited AND NOT in_task;
