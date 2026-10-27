CREATE TABLE chat_retries (
    actor_id uuid NOT NULL REFERENCES human_accounts(id),
    request_id uuid NOT NULL,
    turn_id uuid NOT NULL REFERENCES chat_turns(id),
    previous_authority uuid REFERENCES chat_authorities(id),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(actor_id,request_id)
);
