ALTER TABLE chat_turns DROP CONSTRAINT chat_turns_status_check;
ALTER TABLE chat_turns ADD CHECK (status IN ('queued','running','retrying','waiting','complete'));
