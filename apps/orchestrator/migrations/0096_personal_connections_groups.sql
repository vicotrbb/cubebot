-- A personal connection holds one person's own account; only agents they manage may use it,
-- each by explicit grant, and it stops working while they are suspended.
ALTER TABLE connections ADD COLUMN personal boolean NOT NULL DEFAULT false;
ALTER TABLE connections ADD CONSTRAINT connections_personal_explicit
    CHECK (NOT personal OR access->>'mode' = 'explicit_grants');

-- Group managers change a group's members, never its grants.
ALTER TABLE human_team_members ADD COLUMN manager boolean NOT NULL DEFAULT false;

-- Groups are local, mirrored from Google Workspace, or filled from a single sign-on groups
-- claim. A group may carry a role its members get; an account whose role came from a group
-- follows the groups it is in.
ALTER TABLE human_teams
    ADD COLUMN source text NOT NULL DEFAULT 'local' CHECK (source IN ('local', 'google', 'oidc')),
    ADD COLUMN external_id text CHECK (external_id IS NULL OR octet_length(external_id) BETWEEN 1 AND 320),
    ADD COLUMN role text CHECK (role IN ('administrator', 'auditor', 'member', 'guest')),
    ADD COLUMN synced_at timestamptz,
    ADD CONSTRAINT human_teams_external CHECK ((source = 'local') = (external_id IS NULL));
DROP INDEX human_teams_active_name;
CREATE UNIQUE INDEX human_teams_active_name ON human_teams(name) WHERE active AND source = 'local';
CREATE UNIQUE INDEX human_teams_active_external ON human_teams(source, external_id) WHERE active AND source <> 'local';
ALTER TABLE human_accounts ADD COLUMN role_from_group boolean NOT NULL DEFAULT false;

-- The verified address of a single sign-on identity, for matching directory group members.
ALTER TABLE oidc_bindings ADD COLUMN email text CHECK (email IS NULL OR octet_length(email) BETWEEN 3 AND 320);
CREATE INDEX oidc_bindings_email ON oidc_bindings(lower(email)) WHERE active AND email IS NOT NULL;

-- Google Workspace directory access: a service account with domain-wide delegation, acting as
-- one Workspace administrator, reads the groups linked here.
CREATE TABLE google_directory (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    enabled boolean NOT NULL DEFAULT false,
    admin_email text CHECK (admin_email IS NULL OR octet_length(admin_email) BETWEEN 3 AND 320),
    customer text NOT NULL DEFAULT 'my_customer' CHECK (octet_length(customer) BETWEEN 1 AND 64),
    -- The service account's JSON key, sealed with the credential key.
    service_account bytea,
    client_email text,
    synced_at timestamptz,
    problem text CHECK (problem IS NULL OR octet_length(problem) <= 2000),
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    CHECK (NOT enabled OR (service_account IS NOT NULL AND admin_email IS NOT NULL))
);
INSERT INTO google_directory (singleton) VALUES (true);
