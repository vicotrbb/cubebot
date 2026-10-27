-- Tool eligibility is a grant property, not a command budget.
ALTER TABLE chat_authorities ADD COLUMN terminal_enabled boolean NOT NULL DEFAULT false;
UPDATE chat_authorities SET terminal_enabled = (terminal_budget = 8);
ALTER TABLE chat_authorities DROP COLUMN terminal_budget;
