-- What every model response cost (docs/adr/agents.md#agent-usage-and-monthly-budgets). A row is
-- what the provider reported; a price turns tokens into an estimate, and a provider-reported
-- cost wins. An unpriced row keeps a NULL cost: it is never guessed as zero.
CREATE FUNCTION price_key(model text) RETURNS text
    LANGUAGE sql IMMUTABLE
    RETURN regexp_replace(replace(model, '.', '-'), '-\d{4}-?\d{2}-?\d{2}$', '');

CREATE TABLE model_prices (
    provider text NOT NULL,
    key text NOT NULL,
    input_per_token double precision NOT NULL CHECK (input_per_token >= 0),
    cached_per_token double precision NOT NULL CHECK (cached_per_token >= 0),
    output_per_token double precision NOT NULL CHECK (output_per_token >= 0),
    fetched_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (provider, key)
);

CREATE FUNCTION usage_cost(provider_name text, model text, input bigint, cached bigint, output bigint)
    RETURNS double precision
    LANGUAGE sql STABLE
    RETURN (
        SELECT (input - cached) * p.input_per_token + cached * p.cached_per_token + output * p.output_per_token
        FROM model_prices p
        WHERE p.provider = provider_name AND p.key = price_key(model)
    );

CREATE TABLE usage_events (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    -- NULL for the room coordinator, the orchestrator's own model duty.
    agent_id uuid REFERENCES agents(id),
    turn_id uuid REFERENCES chat_turns(id),
    event_sequence bigint,
    dream_run_id uuid REFERENCES dream_runs(id),
    kind text NOT NULL CHECK (kind IN ('turn', 'dream', 'coordinator')),
    provider text NOT NULL,
    model text NOT NULL,
    input_tokens bigint NOT NULL CHECK (input_tokens >= 0),
    cached_input_tokens bigint NOT NULL DEFAULT 0 CHECK (cached_input_tokens >= 0),
    output_tokens bigint NOT NULL CHECK (output_tokens >= 0),
    cost_usd double precision CHECK (cost_usd >= 0),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (turn_id, event_sequence),
    UNIQUE (dream_run_id)
);
CREATE INDEX usage_events_agent ON usage_events(agent_id, created_at);
CREATE INDEX usage_events_time ON usage_events(created_at);
CREATE INDEX usage_events_unpriced ON usage_events(id) WHERE cost_usd IS NULL;

-- Which model a turn ran on, as it was when the turn was claimed.
ALTER TABLE chat_turns ADD COLUMN provider text, ADD COLUMN model text;

-- Earlier turns recorded tokens but not their model, so they take the installation's current one.
INSERT INTO usage_events (agent_id, turn_id, event_sequence, kind, provider, model, input_tokens, output_tokens, created_at)
SELECT t.conversation_id, e.turn_id, e.sequence, 'turn',
    COALESCE((SELECT c.provider FROM provider_connection c LIMIT 1), 'unknown'),
    COALESCE((SELECT c.model FROM provider_connection c LIMIT 1), 'unknown'),
    (e.payload->>'input_tokens')::bigint, (e.payload->>'output_tokens')::bigint, e.created_at
FROM turn_events e
JOIN chat_turns t ON t.id = e.turn_id
WHERE e.payload->>'kind' = 'usage';

-- Unset means no limit. Past it new model work pauses, as `budget_exhausted`, until it is raised.
ALTER TABLE agents ADD COLUMN monthly_budget_usd double precision CHECK (monthly_budget_usd > 0);

-- When a turn reached its status, so a turn that fails late counts as news when it fails.
ALTER TABLE chat_turns ADD COLUMN status_at timestamptz;
UPDATE chat_turns SET status_at = COALESCE(completed_at, created_at);
ALTER TABLE chat_turns
    ALTER COLUMN status_at SET NOT NULL,
    ALTER COLUMN status_at SET DEFAULT clock_timestamp();
CREATE FUNCTION chat_turn_status_at() RETURNS trigger
    LANGUAGE plpgsql AS $$
BEGIN
    NEW.status_at = clock_timestamp();
    RETURN NEW;
END $$;
CREATE TRIGGER chat_turn_status_at BEFORE UPDATE OF status ON chat_turns
    FOR EACH ROW WHEN (OLD.status IS DISTINCT FROM NEW.status)
    EXECUTE FUNCTION chat_turn_status_at();

-- Up to when each person has seen their activity inbox.
ALTER TABLE human_accounts ADD COLUMN activity_read_at timestamptz;
