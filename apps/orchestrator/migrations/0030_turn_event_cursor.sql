-- A conversation-wide cursor for replaying and streaming turn events to browsers.
-- Recording locks the conversation, so ids become visible in increasing order per conversation.
ALTER TABLE turn_events ADD COLUMN id bigint GENERATED ALWAYS AS IDENTITY UNIQUE;
