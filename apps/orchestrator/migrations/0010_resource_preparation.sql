ALTER TABLE agent_resources ADD COLUMN source_manifest jsonb;
UPDATE agent_resources SET source_manifest = manifest;
ALTER TABLE agent_resources ALTER COLUMN source_manifest SET NOT NULL;
ALTER TABLE agent_resources ADD COLUMN prepared boolean NOT NULL DEFAULT false;
-- Existing dispatched attempts retain their exact recorded manifest and identity.
UPDATE agent_resources SET prepared = true WHERE phase IN ('creating', 'bound') OR uid IS NOT NULL;
ALTER TABLE agent_resources ADD CONSTRAINT resource_prepared_before_dispatch
    CHECK (phase NOT IN ('creating', 'bound') OR prepared);
