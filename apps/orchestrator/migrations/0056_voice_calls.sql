-- Voice calls (docs/adr/console.md#voice-calls-over-the-chat-pipeline). A call's turns are ordinary
-- chat turns; the conversation shows the `calls` row in their place, ordered by the same sequence.
CREATE TABLE calls (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    conversation_id uuid NOT NULL REFERENCES agents(id),
    actor_id uuid NOT NULL REFERENCES human_accounts(id),
    sequence bigint NOT NULL UNIQUE DEFAULT nextval(pg_get_serial_sequence('chat_turns', 'sequence')),
    stt_provider text NOT NULL CHECK (stt_provider IN ('embedded', 'openai', 'elevenlabs')),
    tts_provider text NOT NULL CHECK (tts_provider IN ('embedded', 'openai', 'elevenlabs')),
    voice text NOT NULL CHECK (voice IN ('male', 'female')),
    started_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    ended_at timestamptz
);
CREATE INDEX calls_history ON calls (conversation_id, sequence);

ALTER TABLE chat_turns ADD COLUMN call_id uuid REFERENCES calls(id);
CREATE INDEX chat_turns_call ON chat_turns (call_id) WHERE call_id IS NOT NULL;

CREATE TABLE speech_settings (
    singleton boolean PRIMARY KEY DEFAULT true CHECK (singleton),
    stt_provider text NOT NULL DEFAULT 'embedded' CHECK (stt_provider IN ('embedded', 'openai', 'elevenlabs')),
    tts_provider text NOT NULL DEFAULT 'embedded' CHECK (tts_provider IN ('embedded', 'openai', 'elevenlabs')),
    voice text NOT NULL DEFAULT 'female' CHECK (voice IN ('male', 'female')),
    elevenlabs_credential bytea
);
INSERT INTO speech_settings DEFAULT VALUES;

-- NULL inherits the installation's setting.
ALTER TABLE agents
    ADD COLUMN stt_provider text CHECK (stt_provider IN ('embedded', 'openai', 'elevenlabs')),
    ADD COLUMN tts_provider text CHECK (tts_provider IN ('embedded', 'openai', 'elevenlabs')),
    ADD COLUMN voice text CHECK (voice IN ('male', 'female'));
