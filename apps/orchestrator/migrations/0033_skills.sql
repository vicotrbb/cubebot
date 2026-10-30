-- The installation's shared skill library (ADRs 0055 to 0057). One installation per database, so
-- every skill is installation-wide; `revision` is the current one, and revisions are immutable.
CREATE TABLE skills (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL UNIQUE CHECK (name ~ '^[a-z0-9]+(-[a-z0-9]+)*$' AND char_length(name) <= 64),
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

-- A package directory as relative path to text content, with SKILL.md at its root. Each revision
-- has exactly one author: a person, or the dream run that proposed it.
CREATE TABLE skill_revisions (
    skill_id uuid NOT NULL REFERENCES skills(id),
    revision bigint NOT NULL,
    description text NOT NULL CHECK (char_length(description) BETWEEN 1 AND 1024),
    files jsonb NOT NULL CHECK (jsonb_typeof(files) = 'object' AND files ? 'SKILL.md'),
    human_id uuid REFERENCES human_accounts(id),
    dream_run_id uuid REFERENCES dream_runs(id),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (skill_id, revision),
    CHECK (num_nonnulls(human_id, dream_run_id) = 1)
);
CREATE INDEX skill_revisions_dream ON skill_revisions(dream_run_id) WHERE dream_run_id IS NOT NULL;

-- The revision each agent uses. A new revision never moves another agent's installation.
CREATE TABLE skill_installations (
    agent_id uuid NOT NULL REFERENCES agents(id),
    skill_id uuid NOT NULL,
    revision bigint NOT NULL,
    installed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (agent_id, skill_id),
    FOREIGN KEY (skill_id, revision) REFERENCES skill_revisions(skill_id, revision)
);
