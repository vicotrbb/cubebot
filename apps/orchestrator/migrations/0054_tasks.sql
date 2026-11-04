-- Tasks (docs/adr/rooms.md#tasks-and-the-task-orchestrator): something a person wants done. The
-- task orchestrator picks the agents and gives each its items; they all talk in a room that exists
-- only for the task and appears only on the Tasks page.
ALTER TABLE rooms ADD COLUMN task boolean NOT NULL DEFAULT false;

-- A task room's messages may come from the orchestrator or an agent. `actor_id` stays the person
-- the task belongs to, whose authority the work carries.
ALTER TABLE room_messages
    ADD COLUMN orchestrator boolean NOT NULL DEFAULT false,
    ADD COLUMN author_agent_id uuid REFERENCES agents(id),
    ADD CHECK (NOT (orchestrator AND author_agent_id IS NOT NULL));

CREATE FUNCTION room_message_author(orchestrator boolean, agent uuid, actor uuid) RETURNS text
    LANGUAGE sql STABLE AS $$
    SELECT CASE
        WHEN orchestrator THEN 'Orchestrator'
        WHEN agent IS NOT NULL THEN (SELECT name FROM agents WHERE id = agent)
        ELSE (SELECT login FROM human_accounts WHERE id = actor)
    END
$$;

CREATE TABLE tasks (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    room_id uuid NOT NULL UNIQUE REFERENCES rooms(id),
    created_by uuid NOT NULL REFERENCES human_accounts(id),
    request_id uuid NOT NULL,
    request text NOT NULL CHECK (octet_length(request) BETWEEN 1 AND 16384),
    -- Given by the orchestrator; NULL until its first answer.
    title text CHECK (octet_length(title) BETWEEN 1 AND 128),
    status text NOT NULL DEFAULT 'working' CHECK (status IN ('working', 'needs_you', 'done', 'stopped')),
    -- Assigning rounds since the person last spoke, bounded by task_settings.max_rounds.
    rounds integer NOT NULL DEFAULT 0,
    -- Failed orchestrator calls in a row.
    attempts smallint NOT NULL DEFAULT 0,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (created_by, request_id)
);
CREATE INDEX tasks_recent ON tasks(updated_at DESC);

CREATE TABLE task_items (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    task_id uuid NOT NULL REFERENCES tasks(id),
    agent_id uuid NOT NULL REFERENCES agents(id),
    -- The number the agent uses for it; unique in the task.
    position integer NOT NULL,
    title text NOT NULL CHECK (octet_length(title) BETWEEN 1 AND 1024),
    status text NOT NULL DEFAULT 'todo' CHECK (status IN ('todo', 'doing', 'done', 'skipped')),
    note text CHECK (octet_length(note) <= 2048),
    UNIQUE (task_id, position)
);

-- One row. A NULL model means the orchestrator thinks with the connection the agents use.
CREATE TABLE task_settings (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    provider text CHECK (provider IN ('openai', 'anthropic', 'openrouter', 'gemini', 'grok')),
    model text CHECK (octet_length(model) BETWEEN 1 AND 128),
    max_agents integer NOT NULL DEFAULT 3 CHECK (max_agents BETWEEN 1 AND 16),
    max_rounds integer NOT NULL DEFAULT 3 CHECK (max_rounds BETWEEN 1 AND 16),
    instructions text NOT NULL DEFAULT '' CHECK (octet_length(instructions) <= 8192),
    revision bigint NOT NULL DEFAULT 1,
    CHECK ((provider IS NULL) = (model IS NULL))
);
INSERT INTO task_settings DEFAULT VALUES;
