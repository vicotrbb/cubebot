CREATE INDEX chat_turns_scheduled ON chat_turns (routine_id, created_at) WHERE origin = 'schedule';
