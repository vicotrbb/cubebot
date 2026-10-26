-- One installation per configured database. Bootstrap is serialized on this row.
CREATE TABLE installation (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    id uuid NOT NULL UNIQUE DEFAULT gen_random_uuid(),
    bootstrapped boolean NOT NULL DEFAULT false,
    created_at timestamptz NOT NULL DEFAULT now()
);
INSERT INTO installation (singleton) VALUES (true);

CREATE TABLE human_accounts (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    login text NOT NULL UNIQUE CHECK (login ~ '^[a-z0-9][a-z0-9._-]{2,63}$'),
    password_hash text,
    role text NOT NULL CHECK (role IN ('administrator', 'member')),
    active boolean NOT NULL DEFAULT true,
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE human_sessions (
    token_hash bytea PRIMARY KEY CHECK (octet_length(token_hash) = 32),
    account_id uuid NOT NULL REFERENCES human_accounts(id),
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (expires_at > created_at)
);
CREATE INDEX human_sessions_account ON human_sessions(account_id);

CREATE TABLE identity_audit (
    sequence bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    actor_id uuid REFERENCES human_accounts(id),
    subject_id uuid NOT NULL REFERENCES human_accounts(id),
    action text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);
