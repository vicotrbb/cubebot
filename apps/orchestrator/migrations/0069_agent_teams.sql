-- Agent teams (docs/adr/agents.md#agent-teams): agents grouped under one manager, with shared
-- text every member's turns carry and a room whose agents follow the roster. Teams nest: a child
-- team's manager reports to a member of the parent team and sits in the parent's room.
CREATE TABLE teams (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name text NOT NULL CHECK (length(btrim(name)) BETWEEN 1 AND 128),
    description text NOT NULL DEFAULT '' CHECK (octet_length(description) <= 2048),
    instructions text NOT NULL DEFAULT '' CHECK (octet_length(instructions) <= 8192),
    parent_id uuid REFERENCES teams(id),
    -- The parent team's member this team's manager reports to.
    reports_to uuid,
    room_id uuid NOT NULL UNIQUE REFERENCES rooms(id),
    created_by uuid NOT NULL REFERENCES human_accounts(id),
    request_id uuid NOT NULL,
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    -- A deleted team keeps its row for the tasks that name it; its room is archived.
    deleted_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    UNIQUE (created_by, request_id),
    CHECK ((parent_id IS NULL) = (reports_to IS NULL)),
    CHECK (parent_id <> id)
);
CREATE UNIQUE INDEX teams_live_name ON teams(lower(btrim(name))) WHERE deleted_at IS NULL;
CREATE INDEX teams_parent ON teams(parent_id) WHERE parent_id IS NOT NULL;

CREATE TABLE team_members (
    team_id uuid NOT NULL REFERENCES teams(id),
    agent_id uuid NOT NULL REFERENCES agents(id),
    title text NOT NULL CHECK (length(btrim(title)) BETWEEN 1 AND 64),
    role text NOT NULL DEFAULT '' CHECK (octet_length(role) <= 4096),
    -- NULL for the team's one manager.
    reports_to uuid,
    PRIMARY KEY (team_id, agent_id),
    FOREIGN KEY (team_id, reports_to) REFERENCES team_members(team_id, agent_id)
        DEFERRABLE INITIALLY DEFERRED,
    CHECK (reports_to <> agent_id)
);
CREATE UNIQUE INDEX team_members_manager ON team_members(team_id) WHERE reports_to IS NULL;
CREATE INDEX team_members_agent ON team_members(agent_id);

-- Deferred: a save replaces a whole roster, and the lines must hold when it commits.
ALTER TABLE teams ADD FOREIGN KEY (parent_id, reports_to) REFERENCES team_members(team_id, agent_id)
    DEFERRABLE INITIALLY DEFERRED;

-- An archived room is on no list and takes no messages; administrators can still read it.
ALTER TABLE rooms ADD COLUMN archived boolean NOT NULL DEFAULT false;

-- The team the task orchestrator gave the task to; its manager leads the work.
ALTER TABLE tasks ADD COLUMN team_id uuid REFERENCES teams(id);

-- The agents a team room holds: the team's members and the managers of the teams under it.
CREATE VIEW team_room_agents AS
    SELECT t.room_id, m.agent_id
    FROM teams t JOIN team_members m ON m.team_id=t.id
    WHERE t.deleted_at IS NULL
    UNION
    SELECT p.room_id, m.agent_id
    FROM teams c
    JOIN teams p ON p.id=c.parent_id AND p.deleted_at IS NULL
    JOIN team_members m ON m.team_id=c.id AND m.reports_to IS NULL
    WHERE c.deleted_at IS NULL;
