-- One knowledge pool per agent (ADR 0004). Removal hides a memory from retrieval; its
-- revisions stay as evidence (ADRs 0020, 0026).
CREATE TABLE memories (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id uuid NOT NULL REFERENCES agents(id),
    content text NOT NULL CHECK (octet_length(content) BETWEEN 1 AND 4096),
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    deleted boolean NOT NULL DEFAULT false,
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    search tsvector GENERATED ALWAYS AS (to_tsvector('english', content)) STORED
);
CREATE INDEX memories_agent ON memories(agent_id, updated_at DESC) WHERE NOT deleted;
CREATE INDEX memories_search ON memories USING gin(search) WHERE NOT deleted;

-- Every revision, including the first and a removal, by exactly one author: a person, or the
-- agent through the turn it completed.
CREATE TABLE memory_revisions (
    memory_id uuid NOT NULL REFERENCES memories(id),
    revision bigint NOT NULL,
    content text NOT NULL,
    deleted boolean NOT NULL,
    human_id uuid REFERENCES human_accounts(id),
    turn_id uuid REFERENCES chat_turns(id),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (memory_id, revision),
    CHECK (num_nonnulls(human_id, turn_id) = 1)
);
CREATE INDEX memory_revisions_turn ON memory_revisions(turn_id) WHERE turn_id IS NOT NULL;
