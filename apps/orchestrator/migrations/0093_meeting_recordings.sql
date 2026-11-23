CREATE TABLE meeting_recordings (
    id uuid PRIMARY KEY,
    meeting_id uuid NOT NULL REFERENCES meetings(id) ON DELETE CASCADE,
    started_by uuid NOT NULL REFERENCES human_accounts(id),
    stopped_by uuid REFERENCES human_accounts(id),
    state text NOT NULL DEFAULT 'recording' CHECK (state IN ('recording', 'processing', 'ready', 'failed')),
    progress smallint NOT NULL DEFAULT 0 CHECK (progress BETWEEN 0 AND 100),
    started_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    stopped_at timestamptz,
    finished_at timestamptz,
    duration_ms bigint CHECK (duration_ms >= 0),
    file_id uuid REFERENCES files(id) ON DELETE SET NULL,
    error text CHECK (char_length(error) <= 500),
    CHECK ((state = 'recording') = (stopped_at IS NULL)),
    CHECK ((state IN ('ready', 'failed')) = (finished_at IS NOT NULL))
);
CREATE INDEX meeting_recordings_meeting ON meeting_recordings (meeting_id, started_at);
CREATE INDEX meeting_recordings_pending ON meeting_recordings (started_at) WHERE state IN ('recording', 'processing');
