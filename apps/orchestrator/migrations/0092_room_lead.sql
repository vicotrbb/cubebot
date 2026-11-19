ALTER TABLE rooms ADD COLUMN lead_agent_id uuid REFERENCES agents(id) ON DELETE SET NULL;
