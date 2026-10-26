-- Password-bearing commands retain only a slow one-way verifier. Their audit
-- payload and normal request digest never contain the password or a fast digest.
ALTER TABLE identity_commands ADD COLUMN secret_verifier text;
ALTER TABLE invitations ADD COLUMN revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0);
ALTER TABLE invitations ADD COLUMN accept_request_id uuid;
