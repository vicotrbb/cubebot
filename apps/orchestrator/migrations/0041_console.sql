-- Console state (docs/design/console.md): the agent's cube avatar, a person's console preferences
-- and their read cursors per conversation or room. None of it is part of definition revisions.
ALTER TABLE agents
    ADD COLUMN avatar jsonb NOT NULL DEFAULT '{"paint":"orange","eyes":"round","accessory":"none"}'
    CHECK (
        avatar->>'paint' IN ('orange', 'sage', 'paper', 'wood', 'sky', 'rose', 'lemon', 'plum')
        AND avatar->>'eyes' IN ('round', 'wide', 'sleepy', 'dot')
        AND avatar->>'accessory' IN ('none', 'cap', 'glasses', 'bow', 'antenna')
    );

CREATE TABLE account_preferences (
    account_id uuid PRIMARY KEY REFERENCES human_accounts(id),
    document jsonb NOT NULL CHECK (jsonb_typeof(document) = 'object'),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

-- The highest sequence a person has read; a lower write never moves it back.
CREATE TABLE read_cursors (
    account_id uuid NOT NULL REFERENCES human_accounts(id),
    scope_kind text NOT NULL CHECK (scope_kind IN ('conversation', 'room')),
    scope_id uuid NOT NULL,
    sequence bigint NOT NULL CHECK (sequence >= 0),
    PRIMARY KEY (account_id, scope_kind, scope_id)
);
