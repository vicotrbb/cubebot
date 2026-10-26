CREATE TABLE invitations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    login text NOT NULL CHECK (login ~ '^[a-z0-9][a-z0-9._-]{2,63}$'),
    token_hash bytea NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32),
    created_by uuid NOT NULL REFERENCES human_accounts(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    expires_at timestamptz NOT NULL DEFAULT now() + interval '7 days',
    revoked_by uuid REFERENCES human_accounts(id),
    accepted_by uuid UNIQUE REFERENCES human_accounts(id),
    accepted_at timestamptz,
    CHECK ((accepted_by IS NULL) = (accepted_at IS NULL)),
    CHECK (accepted_by IS NULL OR revoked_by IS NULL)
);
CREATE UNIQUE INDEX invitations_pending_login ON invitations(login)
    WHERE accepted_by IS NULL AND revoked_by IS NULL;
