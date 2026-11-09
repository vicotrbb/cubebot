-- A routine's schedule is five-field cron in `time_zone`, an IANA zone name; existing routines keep
-- reading theirs in UTC (docs/adr/agents.md#routines).
ALTER TABLE routines
    ADD COLUMN time_zone text NOT NULL DEFAULT 'UTC' CHECK (octet_length(time_zone) BETWEEN 1 AND 64);
