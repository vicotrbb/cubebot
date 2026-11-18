ALTER TABLE agents
    ADD COLUMN personality text NOT NULL DEFAULT '' CHECK (char_length(personality) <= 4000),
    ADD COLUMN personality_updated_at timestamptz,
    ADD COLUMN personality_edited_by uuid REFERENCES human_accounts(id);
