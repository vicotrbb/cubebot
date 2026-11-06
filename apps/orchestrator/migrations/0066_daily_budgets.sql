-- A per-agent cap for one UTC day beside the monthly one (docs/adr/agents.md#agent-usage-and-monthly-budgets).
-- Unset means no daily limit; either cap pauses new model work as `budget_exhausted`.
ALTER TABLE agents ADD COLUMN daily_budget_usd double precision CHECK (daily_budget_usd > 0);
