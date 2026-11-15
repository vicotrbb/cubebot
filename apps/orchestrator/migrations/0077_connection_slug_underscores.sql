ALTER TABLE connections DROP CONSTRAINT connections_slug_check;
ALTER TABLE connections ADD CONSTRAINT connections_slug_check
    CHECK (slug ~ '^[a-z0-9][a-z0-9_-]{0,31}$' AND slug !~ '__' AND slug !~ '_$');
