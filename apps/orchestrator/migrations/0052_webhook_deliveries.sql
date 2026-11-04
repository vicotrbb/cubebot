-- routine_hooks is now also the routine's delivery history, kept 30 days. A key still replays for
-- one day: older rows lose it. Limited deliveries are not recorded: writing a row for each would
-- let a flood fill the table the limit protects.
ALTER TABLE routine_hooks
    ADD COLUMN outcome text NOT NULL DEFAULT 'accepted'
        CHECK (outcome IN ('accepted', 'coalesced', 'replayed', 'paused')),
    ADD COLUMN bytes integer NOT NULL DEFAULT 0 CHECK (bytes >= 0);
UPDATE routine_hooks SET outcome = 'coalesced' WHERE turn_id IS NULL;
