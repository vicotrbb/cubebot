-- Routines (ADR 0087): a stored prompt and/or command for one agent. Every occurrence is a turn
-- in the agent's conversation carrying the routine and what started it.
CREATE TABLE routines (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id uuid NOT NULL REFERENCES agents(id),
    name text NOT NULL CHECK (octet_length(name) BETWEEN 1 AND 128),
    prompt text CHECK (octet_length(prompt) BETWEEN 1 AND 8192),
    command text CHECK (octet_length(command) BETWEEN 1 AND 4096),
    -- Five-field cron in UTC; NULL runs only on demand or by webhook.
    schedule text CHECK (octet_length(schedule) BETWEEN 1 AND 128),
    webhook_secret_hash bytea,
    coalesce boolean NOT NULL DEFAULT true,
    enabled boolean NOT NULL DEFAULT true,
    -- The next scheduled run; NULL without a schedule.
    next_at timestamptz,
    last_run_at timestamptz,
    revision bigint NOT NULL DEFAULT 1,
    -- The person occurrences act as when nobody runs them by hand.
    created_by uuid NOT NULL REFERENCES human_accounts(id),
    -- A person's edit makes an agent's routine theirs, so the agent cannot undo it.
    managed_by text NOT NULL CHECK (managed_by IN ('person','agent')),
    request_id uuid NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (agent_id, request_id),
    CHECK (prompt IS NOT NULL OR command IS NOT NULL)
);
CREATE INDEX routines_agent ON routines(agent_id, name);
CREATE INDEX routines_due ON routines(next_at) WHERE enabled;

ALTER TABLE chat_turns
    ADD COLUMN routine_id uuid REFERENCES routines(id) ON DELETE SET NULL,
    ADD COLUMN origin text CHECK (origin IN ('person','agent','schedule','webhook')),
    -- Copied from the routine when the occurrence is made, so an edit never changes queued work.
    ADD COLUMN command text,
    ADD COLUMN command_only boolean NOT NULL DEFAULT false,
    ADD CHECK (routine_id IS NULL OR origin IS NOT NULL),
    ADD CHECK (NOT command_only OR command IS NOT NULL);
CREATE INDEX chat_turns_routine ON chat_turns(routine_id, sequence) WHERE routine_id IS NOT NULL;

-- Authenticated webhook deliveries of the last day: the rate window and Idempotency-Key replay.
CREATE TABLE routine_hooks (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    routine_id uuid NOT NULL REFERENCES routines(id) ON DELETE CASCADE,
    key text CHECK (octet_length(key) BETWEEN 1 AND 255),
    turn_id uuid REFERENCES chat_turns(id) ON DELETE SET NULL,
    received_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE UNIQUE INDEX routine_hooks_key ON routine_hooks(routine_id, key) WHERE key IS NOT NULL;
CREATE INDEX routine_hooks_received ON routine_hooks(routine_id, received_at);
