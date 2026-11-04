-- A person's profile picture: a JPEG or PNG the console resizes before upload. The revision
-- changes with every upload or removal, so image URLs carrying it can be cached.
ALTER TABLE human_accounts
    ADD COLUMN avatar bytea CHECK (octet_length(avatar) BETWEEN 1 AND 1048576),
    ADD COLUMN avatar_revision bigint NOT NULL DEFAULT 0 CHECK (avatar_revision >= 0);
