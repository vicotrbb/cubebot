-- A revoked sender's turn now fails instead of waiting; those already waiting hold their lanes.
UPDATE chat_turns
SET status = 'failed'
WHERE status = 'waiting' AND error = 'sender_access_revoked';
