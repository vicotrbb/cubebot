-- Cubebot Meetings, phase 1 (docs/adr/meetings.md): the records the orchestrator owns. Media is not
-- here; a meeting is a room, its people, its links, its guests' tokens and what was said.
CREATE TABLE meeting_settings (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    lobby_default boolean NOT NULL DEFAULT true,
    consent_notice boolean NOT NULL DEFAULT true,
    agent_mode text NOT NULL DEFAULT 'on_mention'
        CHECK (agent_mode IN ('listen_only', 'on_mention', 'proactive'))
);
INSERT INTO meeting_settings DEFAULT VALUES;

CREATE TABLE meetings (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    created_by uuid NOT NULL REFERENCES human_accounts(id),
    title text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 120),
    -- Copied from meeting_settings at creation, so a later settings change never alters a meeting.
    consent_notice boolean NOT NULL,
    agent_mode text NOT NULL CHECK (agent_mode IN ('listen_only', 'on_mention', 'proactive')),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    ended_at timestamptz
);
CREATE INDEX meetings_owner ON meetings (created_by, created_at DESC);

-- Only the SHA-256 of a link's token is stored; the token is shown once, at creation.
CREATE TABLE meeting_links (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    meeting_id uuid NOT NULL REFERENCES meetings(id),
    token_digest bytea NOT NULL UNIQUE,
    lobby boolean NOT NULL,
    -- The last moment a new guest can join; guests already in are not affected by it.
    expires_at timestamptz,
    revoked_at timestamptz,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX meeting_links_meeting ON meeting_links (meeting_id);

CREATE TABLE meeting_participants (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    meeting_id uuid NOT NULL REFERENCES meetings(id),
    kind text NOT NULL CHECK (kind IN ('agent', 'guest')),
    agent_id uuid REFERENCES agents(id),
    -- The link a guest came in through; revoking it ends that guest's access.
    link_id uuid REFERENCES meeting_links(id),
    name text NOT NULL CHECK (char_length(name) BETWEEN 1 AND 64),
    status text NOT NULL CHECK (status IN ('invited', 'waiting', 'admitted', 'denied')),
    agent_mode text CHECK (agent_mode IN ('listen_only', 'on_mention', 'proactive')),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK ((kind = 'agent' AND agent_id IS NOT NULL AND link_id IS NULL AND agent_mode IS NOT NULL
            AND status = 'invited')
        OR (kind = 'guest' AND agent_id IS NULL AND link_id IS NOT NULL AND agent_mode IS NULL
            AND status <> 'invited'))
);
CREATE UNIQUE INDEX meeting_participants_agent ON meeting_participants (meeting_id, agent_id)
    WHERE agent_id IS NOT NULL;
CREATE INDEX meeting_participants_meeting ON meeting_participants (meeting_id, created_at);

-- What lets a guest, and only a guest, reach one meeting. These are not console sessions: nothing
-- that authenticates a person reads this table, and nothing here authenticates a person.
CREATE TABLE meeting_join_tokens (
    token_digest bytea PRIMARY KEY,
    participant_id uuid NOT NULL REFERENCES meeting_participants(id),
    -- `lobby` may only ask whether the host has admitted the guest; `media` may join the call.
    scope text NOT NULL CHECK (scope IN ('lobby', 'media')),
    expires_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX meeting_join_tokens_participant ON meeting_join_tokens (participant_id, expires_at);

CREATE TABLE meeting_transcript_segments (
    sequence bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    meeting_id uuid NOT NULL REFERENCES meetings(id),
    participant_id uuid REFERENCES meeting_participants(id),
    -- The name as spoken in the meeting; the participant may be unknown or a person without a row.
    speaker text NOT NULL CHECK (char_length(speaker) BETWEEN 1 AND 64),
    spoken_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    text text NOT NULL CHECK (char_length(text) BETWEEN 1 AND 4096)
);
CREATE INDEX meeting_transcript_meeting ON meeting_transcript_segments (meeting_id, sequence);
