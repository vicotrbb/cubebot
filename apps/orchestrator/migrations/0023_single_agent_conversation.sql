-- One permanent conversation is intrinsic to an agent, including agents inserted directly.
-- Preserve old IDs/provenance for audit, but never retain independent active histories.
ALTER TABLE conversations RENAME TO conversation_origins;
ALTER TABLE chat_turns ADD COLUMN origin_conversation_id uuid REFERENCES conversation_origins(id);
UPDATE chat_turns SET origin_conversation_id=conversation_id;
ALTER TABLE access_grants ADD COLUMN origin_conversation_id uuid REFERENCES conversation_origins(id);
UPDATE access_grants g SET origin_conversation_id=o.id FROM conversation_origins o
WHERE g.scope_kind='conversation' AND g.scope_id=o.id;
ALTER TABLE chat_turns DROP CONSTRAINT chat_turns_conversation_id_fkey;
UPDATE chat_turns t SET conversation_id=o.agent_id FROM conversation_origins o WHERE t.conversation_id=o.id;
ALTER TABLE chat_turns ADD FOREIGN KEY (conversation_id) REFERENCES agents(id);

-- Separate former audiences must not gain access to one another's merged transcripts.
-- Keep revoked grants as evidence. Administrators can review and grant the unified history.
UPDATE access_grants g SET revoked_at=clock_timestamp(),revision=revision+1
FROM conversation_origins o
WHERE g.scope_kind='conversation' AND g.scope_id=o.id AND g.revoked_at IS NULL
AND (SELECT count(*) FROM conversation_origins s WHERE s.agent_id=o.agent_id)>1;

-- Avoid duplicate active grants when a canonical-ID grant already exists.
UPDATE access_grants g SET revoked_at=clock_timestamp(),revision=revision+1
FROM conversation_origins o
WHERE g.scope_kind='conversation' AND g.scope_id=o.id AND g.revoked_at IS NULL
AND EXISTS (SELECT 1 FROM access_grants n WHERE n.id<>g.id AND n.scope_kind='conversation'
    AND n.scope_id=o.agent_id AND n.capability=g.capability AND n.revoked_at IS NULL
    AND n.account_id IS NOT DISTINCT FROM g.account_id AND n.team_id IS NOT DISTINCT FROM g.team_id);
UPDATE access_grants g SET scope_id=o.agent_id
FROM conversation_origins o WHERE g.scope_kind='conversation' AND g.scope_id=o.id;

-- Preserve the original initiator's rights only when that person already saw every source.
INSERT INTO access_grants (account_id,scope_kind,scope_id,capability)
SELECT min(o.created_by::text)::uuid,'conversation',o.agent_id,c.capability
FROM conversation_origins o CROSS JOIN (VALUES ('conversation.read'),('conversation.send')) c(capability)
GROUP BY o.agent_id,c.capability HAVING count(DISTINCT o.created_by)=1
ON CONFLICT DO NOTHING;

-- The view has exactly one row per agent by construction, with the same stable identity.
CREATE VIEW conversations AS SELECT id,id AS agent_id,created_by FROM agents;
