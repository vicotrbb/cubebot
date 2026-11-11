ALTER TABLE turn_attachments ADD COLUMN inbound boolean NOT NULL DEFAULT false;

CREATE TABLE room_message_attachments (
    room_message_id uuid NOT NULL REFERENCES room_messages(id),
    file_id uuid NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (room_message_id, file_id)
);
CREATE INDEX room_message_attachments_file ON room_message_attachments(file_id);

ALTER TABLE chat_turns DROP CONSTRAINT chat_turns_message_check,
    ADD CONSTRAINT chat_turns_message_check CHECK (octet_length(message) <= 20480);
ALTER TABLE room_messages DROP CONSTRAINT room_messages_message_check,
    ADD CONSTRAINT room_messages_message_check CHECK (octet_length(message) <= 16384);
