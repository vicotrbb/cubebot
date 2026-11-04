-- Files an agent sent with a turn's reply. Reading the reply grants reading these, so a person
-- with access to the conversation or room needs no separate share. Deleting the file removes it.
CREATE TABLE turn_attachments (
    turn_id uuid NOT NULL REFERENCES chat_turns(id),
    file_id uuid NOT NULL REFERENCES files(id) ON DELETE CASCADE,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (turn_id, file_id)
);
CREATE INDEX turn_attachments_file ON turn_attachments(file_id);
