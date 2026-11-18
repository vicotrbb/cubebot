ALTER TABLE chat_turns
    ADD COLUMN message_deleted boolean NOT NULL DEFAULT false,
    ADD COLUMN response_deleted boolean NOT NULL DEFAULT false;
