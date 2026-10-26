CREATE TABLE installation_kubernetes (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    namespace text NOT NULL CHECK (namespace ~ '^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$'),
    anchor_name text NOT NULL,
    anchor_attempt uuid NOT NULL DEFAULT gen_random_uuid(),
    anchor_uid text CHECK (octet_length(anchor_uid) BETWEEN 1 AND 255),
    anchor_resource_version text,
    CHECK ((anchor_uid IS NULL) = (anchor_resource_version IS NULL))
);
CREATE TABLE agent_resources (
    id uuid PRIMARY KEY,
    agent_id uuid NOT NULL REFERENCES agents(id),
    kind text NOT NULL CHECK (kind IN ('pod', 'workspace')),
    epoch bigint NOT NULL CHECK (epoch >= 0),
    target_generation bigint NOT NULL CHECK (target_generation >= 0),
    definition_revision bigint NOT NULL CHECK (definition_revision > 0),
    namespace text NOT NULL,
    name text NOT NULL,
    manifest jsonb NOT NULL,
    phase text NOT NULL DEFAULT 'planned' CHECK (phase IN ('planned', 'creating', 'bound', 'stopped')),
    uid text CHECK (octet_length(uid) BETWEEN 1 AND 255),
    resource_version text,
    blocked_reason text,
    termination_proof jsonb,
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (agent_id, kind, epoch),
    UNIQUE (namespace, kind, name),
    CHECK ((uid IS NULL) = (resource_version IS NULL)),
    CHECK (phase <> 'bound' OR uid IS NOT NULL),
    CHECK (phase <> 'stopped' OR termination_proof IS NOT NULL),
    CHECK (phase <> 'planned' OR uid IS NULL),
    CHECK (kind <> 'workspace' OR (epoch = 0 AND phase <> 'stopped'))
);
CREATE UNIQUE INDEX agent_resources_one_unresolved ON agent_resources(agent_id, kind) WHERE phase <> 'stopped';
