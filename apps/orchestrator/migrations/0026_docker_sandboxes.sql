-- Cluster-less Docker sandboxes are pod attempts in namespace 'docker' (ADR 0084), so sessions,
-- drains and epochs apply unchanged. uid is the container ID; kubernetes_phase holds the observed
-- container phase. Only a digest of each container's bootstrap token is stored.
CREATE TABLE docker_sandbox_tokens (
    attempt_id uuid PRIMARY KEY REFERENCES agent_resources(id),
    token_hash bytea NOT NULL UNIQUE CHECK (octet_length(token_hash) = 32)
);
