-- Native connections (docs/adr/identity.md#native-connectors): one account on a compiled-in
-- connector. The credential is one Credentials record bound to the connection's id; it holds
-- pasted secrets, an OAuth client secret and tokens, or a GitHub App's key, and never leaves the
-- orchestrator except as a short-lived credential for an agent with access.
CREATE TABLE connections (
    id uuid PRIMARY KEY,
    connector text NOT NULL CHECK (connector ~ '^[a-z0-9-]{1,24}$'),
    -- Prefixes the connection's tools as `<slug>__<tool>`.
    slug text NOT NULL UNIQUE CHECK (slug ~ '^[a-z0-9][a-z0-9-]{0,31}$'),
    -- The manifest's sign-in method: api_key, password, oauth2, github_app or linked.
    auth text NOT NULL CHECK (auth IN ('api_key', 'password', 'oauth2', 'github_app', 'linked')),
    -- The account as its vendor names it, from the last successful check.
    account text CHECK (octet_length(account) BETWEEN 1 AND 320),
    -- Settings that are not secret, such as a server address or the products switched on.
    config jsonb NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(config) = 'object'),
    credential bytea NOT NULL,
    -- When the access token in the credential expires, and when the orchestrator renews it.
    expires_at timestamptz,
    refresh_after timestamptz,
    access jsonb NOT NULL DEFAULT '{"mode":"all_agents"}',
    level text NOT NULL CHECK (level IN ('read', 'write', 'full')),
    disabled_tools jsonb NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(disabled_tools) = 'array'),
    status text NOT NULL DEFAULT 'connected' CHECK (status IN ('connected', 'sign_in_again', 'error')),
    -- Why the status is not `connected`, worded for the console.
    problem text CHECK (octet_length(problem) <= 1024),
    checked_at timestamptz,
    -- The connector's position in its event stream; NULL until the first poll sets a baseline.
    cursor text CHECK (octet_length(cursor) <= 16384),
    polled_at timestamptz,
    created_by uuid NOT NULL REFERENCES human_accounts(id),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX connections_refresh ON connections(refresh_after) WHERE refresh_after IS NOT NULL;

-- Events a poll found, kept 30 days: the console shows them, and an event is acted on once.
CREATE TABLE connection_events (
    connection_id uuid NOT NULL REFERENCES connections(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (octet_length(kind) BETWEEN 1 AND 64),
    event_id text NOT NULL CHECK (octet_length(event_id) BETWEEN 1 AND 512),
    summary text NOT NULL CHECK (octet_length(summary) <= 1024),
    -- Routines it started.
    routines integer NOT NULL DEFAULT 0,
    received_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (connection_id, kind, event_id)
);
CREATE INDEX connection_events_recent ON connection_events(connection_id, received_at DESC);

-- A routine may run when a connection reports an event of one kind, besides its schedule and webhook.
ALTER TABLE routines
    ADD COLUMN trigger_connection uuid REFERENCES connections(id) ON DELETE SET NULL,
    ADD COLUMN trigger_event text CHECK (octet_length(trigger_event) BETWEEN 1 AND 64),
    ADD CHECK ((trigger_connection IS NULL) = (trigger_event IS NULL) OR trigger_connection IS NULL);
CREATE INDEX routines_trigger ON routines(trigger_connection, trigger_event) WHERE trigger_connection IS NOT NULL;

ALTER TABLE chat_turns DROP CONSTRAINT chat_turns_origin_check;
ALTER TABLE chat_turns ADD CONSTRAINT chat_turns_origin_check
    CHECK (origin IN ('person', 'agent', 'schedule', 'webhook', 'event'));
