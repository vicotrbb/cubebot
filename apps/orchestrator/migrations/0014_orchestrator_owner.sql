-- No time-based takeover. A lost session does not prove its process stopped.
CREATE TABLE orchestrator_owner (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    instance_id uuid NOT NULL CHECK (instance_id <> '00000000-0000-0000-0000-000000000000'),
    generation bigint NOT NULL CHECK (generation > 0),
    namespace text NOT NULL CHECK (namespace ~ '^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$'),
    pod_name text NOT NULL CHECK (pod_name ~ '^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$'),
    pod_uid text NOT NULL CHECK (octet_length(pod_uid) BETWEEN 1 AND 255),
    container_id text NOT NULL CHECK (octet_length(container_id) BETWEEN 1 AND 512),
    acquired_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    released_at timestamptz,
    previous_termination_proof jsonb
);
