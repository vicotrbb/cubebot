-- A revision's author is a person, a dream run, or the turn of an agent that wrote it during its
-- work (docs/adr/memory.md#reusable-skills-created-by-humans-agents-and-dreaming). One with no
-- author shipped with the platform.
ALTER TABLE skill_revisions DROP CONSTRAINT skill_revisions_check;
ALTER TABLE skill_revisions
    ADD COLUMN turn_id uuid REFERENCES chat_turns(id),
    ADD CHECK (num_nonnulls(human_id, dream_run_id, turn_id) <= 1);
CREATE INDEX skill_revisions_turn ON skill_revisions(turn_id) WHERE turn_id IS NOT NULL;

-- New agents start with these installed; uninstalling from one agent sticks.
ALTER TABLE skills ADD COLUMN installs_by_default boolean NOT NULL DEFAULT false;
