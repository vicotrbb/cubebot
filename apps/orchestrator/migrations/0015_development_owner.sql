-- Database-only development serving participates in the same exclusion lock.
-- An unclean local process has no Kubernetes termination proof, so retain its claim.
CREATE TABLE orchestrator_development_owner (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    instance_id uuid NOT NULL CHECK (instance_id <> '00000000-0000-0000-0000-000000000000'),
    generation bigint NOT NULL CHECK (generation > 0),
    released_at timestamptz
);
