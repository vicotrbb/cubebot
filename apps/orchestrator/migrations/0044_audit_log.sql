-- One installation audit log for administrators: who did what to which resource, newest first.
-- The command journals stay the receipts for retries; this log is what people read. `detail`
-- holds the non-secret request or outcome, never a password, token or vault secret.
CREATE TABLE audit_log (
    sequence bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    at timestamptz NOT NULL DEFAULT clock_timestamp(),
    -- The person who acted; NULL for an agent or the installation itself.
    actor_id uuid REFERENCES human_accounts(id),
    -- The agent that acted (a broadcast, a vault fill, a login request); NULL for a person.
    agent_id uuid REFERENCES agents(id),
    kind text NOT NULL CHECK (kind ~ '^[a-z]+(_[a-z]+)*\.[a-z]+(_[a-z]+)*$'),
    resource_id uuid,
    detail jsonb NOT NULL DEFAULT '{}' CHECK (jsonb_typeof(detail) = 'object')
);
CREATE INDEX audit_log_kind ON audit_log (kind, sequence);

-- What the three journals already hold, in time order. Duplicated identity_audit actions
-- (account_created, account_suspended, oidc_identity_revoked) are covered by their commands.
INSERT INTO audit_log (at, actor_id, agent_id, kind, resource_id, detail)
SELECT at, actor_id, agent_id, kind, resource_id, detail
FROM (
    SELECT e.created_at AS at, e.actor_id, NULL::uuid AS agent_id, e.kind, e.resource_id,
        c.input_body AS detail
    FROM identity_events e
    JOIN identity_commands c USING (actor_id, kind, request_id)
    UNION ALL
    SELECT e.created_at, e.actor_id, NULL, e.kind, e.agent_id,
        COALESCE(c.input_body -> 'input', '{}')
    FROM agent_events e
    JOIN agent_commands c USING (actor_id, kind, request_id)
    WHERE jsonb_typeof(COALESCE(c.input_body -> 'input', '{}')) = 'object'
    UNION ALL
    SELECT created_at, actor_id, NULL, CASE action
            WHEN 'bootstrap' THEN 'account.bootstrap'
            WHEN 'invitation_accepted' THEN 'invitation.accept'
            WHEN 'oidc_sign_in' THEN 'session.sign_in'
            WHEN 'oidc_identity_linked' THEN 'oidc.link'
        END, subject_id,
        CASE action WHEN 'oidc_sign_in' THEN '{"method":"oidc"}'::jsonb ELSE '{}' END
    FROM identity_audit
    WHERE action IN ('bootstrap', 'invitation_accepted', 'oidc_sign_in', 'oidc_identity_linked')
) journal
ORDER BY at;
