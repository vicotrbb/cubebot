-- Wave 2 sign-ins: the person's own app trading its id and secret for tokens (client_credentials),
-- and connectors with nothing to sign in with, such as public feeds (public).
ALTER TABLE connections DROP CONSTRAINT connections_auth_check;
ALTER TABLE connections ADD CONSTRAINT connections_auth_check CHECK (
    auth IN ('api_key', 'password', 'oauth2', 'github_app', 'linked', 'client_credentials', 'public')
);
