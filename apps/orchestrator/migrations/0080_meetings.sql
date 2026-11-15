CREATE TABLE meeting_settings (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    agents_join boolean NOT NULL DEFAULT true,
    agent_screens boolean NOT NULL DEFAULT true,
    guests boolean NOT NULL DEFAULT true,
    max_participants integer NOT NULL DEFAULT 12 CHECK (max_participants BETWEEN 2 AND 24),
    agent_mode text NOT NULL DEFAULT 'relevant' CHECK (agent_mode IN ('relevant', 'named'))
);
INSERT INTO meeting_settings DEFAULT VALUES;

CREATE TABLE meetings (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    title text NOT NULL CHECK (char_length(title) BETWEEN 1 AND 200),
    description text NOT NULL DEFAULT '' CHECK (char_length(description) <= 4000),
    owner_id uuid NOT NULL REFERENCES human_accounts(id),
    starts_at timestamptz,
    ends_at timestamptz,
    link_token text UNIQUE,
    open_join boolean NOT NULL DEFAULT false,
    locked boolean NOT NULL DEFAULT false,
    agent_mode text NOT NULL CHECK (agent_mode IN ('relevant', 'named')),
    started_at timestamptz,
    ended_at timestamptz,
    summary text,
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK ((starts_at IS NULL) = (ends_at IS NULL)),
    CHECK (ends_at > starts_at)
);
CREATE INDEX meetings_owner ON meetings (owner_id, starts_at);

CREATE TABLE meeting_invitations (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    meeting_id uuid NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    account_id uuid REFERENCES human_accounts(id),
    agent_id uuid REFERENCES agents(id),
    email text CHECK (email = lower(email) AND char_length(email) <= 254),
    invited_by uuid NOT NULL REFERENCES human_accounts(id),
    created_at timestamptz NOT NULL DEFAULT now(),
    email_connection_id uuid REFERENCES connections(id) ON DELETE SET NULL,
    emailed_at timestamptz,
    email_error text,
    CHECK (num_nonnulls(account_id, agent_id, email) = 1),
    UNIQUE (meeting_id, account_id),
    UNIQUE (meeting_id, agent_id)
);
CREATE UNIQUE INDEX meeting_invitations_email ON meeting_invitations (meeting_id, lower(email));
CREATE INDEX meeting_invitations_account ON meeting_invitations (account_id) WHERE account_id IS NOT NULL;

CREATE TABLE meeting_attendance (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    meeting_id uuid NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    account_id uuid REFERENCES human_accounts(id),
    agent_id uuid REFERENCES agents(id),
    guest_name text CHECK (char_length(guest_name) BETWEEN 1 AND 40),
    sequence bigint UNIQUE CHECK (sequence IS NULL OR agent_id IS NOT NULL),
    joined_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    left_at timestamptz,
    CHECK (num_nonnulls(account_id, agent_id, guest_name) = 1)
);
CREATE INDEX meeting_attendance_meeting ON meeting_attendance (meeting_id, joined_at);
CREATE INDEX meeting_attendance_agent ON meeting_attendance (agent_id, sequence) WHERE agent_id IS NOT NULL;

CREATE TABLE meeting_utterances (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    meeting_id uuid NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    sequence bigint GENERATED ALWAYS AS IDENTITY,
    account_id uuid REFERENCES human_accounts(id),
    agent_id uuid REFERENCES agents(id),
    guest_name text CHECK (char_length(guest_name) BETWEEN 1 AND 40),
    text text NOT NULL,
    at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK (num_nonnulls(account_id, agent_id, guest_name) = 1)
);
CREATE INDEX meeting_utterances_meeting ON meeting_utterances (meeting_id, sequence);

ALTER TABLE chat_turns
    ADD COLUMN meeting_id uuid REFERENCES meetings(id) ON DELETE SET NULL,
    ADD COLUMN talk_only boolean NOT NULL DEFAULT false,
    ADD COLUMN meeting_summary boolean NOT NULL DEFAULT false;
CREATE INDEX chat_turns_meeting ON chat_turns (meeting_id) WHERE meeting_id IS NOT NULL;
