CREATE TABLE apps (
    id uuid PRIMARY KEY,
    agent_id uuid NOT NULL REFERENCES agents(id),
    slug text NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9][a-z0-9-]{1,39}$'),
    name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 60),
    description text NOT NULL DEFAULT '' CHECK (char_length(description) <= 300),
    icon text NOT NULL DEFAULT '' CHECK (char_length(icon) <= 16),
    folder text NOT NULL,
    command text,
    port integer CHECK (port BETWEEN 1024 AND 65535 AND port <> 7400),
    tools text[] NOT NULL DEFAULT '{}',
    desired text NOT NULL DEFAULT 'running' CHECK (desired IN ('running', 'stopped')),
    updating boolean NOT NULL DEFAULT false,
    reported text NOT NULL DEFAULT 'starting' CHECK (reported IN ('starting', 'running', 'stopped', 'failed')),
    memory bigint,
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((command IS NULL) = (port IS NULL))
);
