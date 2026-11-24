-- The watchdog (docs/adr/agents.md#a-watchdog-that-notifies): rules over the turn ledger, the
-- audit log and usage open findings for administrators. It never blocks or stops anything.
CREATE TABLE watchdog_findings (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    rule text NOT NULL CHECK (rule ~ '^[a-z]+(_[a-z]+)*$'),
    severity text NOT NULL CHECK (severity IN ('low', 'medium', 'high')),
    agent_id uuid REFERENCES agents(id) ON DELETE SET NULL,
    -- The latest turn that matched, the evidence a person opens in the trace.
    turn_id uuid REFERENCES chat_turns(id) ON DELETE SET NULL,
    summary text NOT NULL CHECK (octet_length(summary) BETWEEN 1 AND 280),
    -- What matched, with secret-shaped values masked; never a credential.
    evidence text CHECK (octet_length(evidence) <= 1024),
    count integer NOT NULL DEFAULT 1 CHECK (count > 0),
    opened_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    seen_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    dismissed_at timestamptz,
    dismissed_by uuid REFERENCES human_accounts(id),
    CHECK ((dismissed_at IS NULL) = (dismissed_by IS NULL))
);
-- A repeat while a finding is open counts on it instead of opening another.
CREATE UNIQUE INDEX watchdog_findings_open
    ON watchdog_findings (rule, COALESCE(agent_id, '00000000-0000-0000-0000-000000000000'))
    WHERE dismissed_at IS NULL;
CREATE INDEX watchdog_findings_dismissed ON watchdog_findings (dismissed_at DESC) WHERE dismissed_at IS NOT NULL;

-- A rule a person silenced, for one agent or (agent NULL) for the whole installation.
CREATE TABLE watchdog_mutes (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    rule text NOT NULL CHECK (rule ~ '^[a-z]+(_[a-z]+)*$'),
    agent_id uuid REFERENCES agents(id) ON DELETE CASCADE,
    muted_by uuid NOT NULL REFERENCES human_accounts(id),
    muted_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE UNIQUE INDEX watchdog_mutes_scope
    ON watchdog_mutes (rule, COALESCE(agent_id, '00000000-0000-0000-0000-000000000000'));

-- How far the watchdog has read. It starts at the present, so history never floods it.
CREATE TABLE watchdog_cursor (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    turn_event bigint NOT NULL,
    audit bigint NOT NULL,
    read_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
INSERT INTO watchdog_cursor (turn_event, audit)
SELECT COALESCE((SELECT max(id) FROM turn_events), 0), COALESCE((SELECT max(sequence) FROM audit_log), 0);
