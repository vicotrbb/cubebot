ALTER TABLE meeting_settings DROP CONSTRAINT meeting_settings_agent_mode_check;
ALTER TABLE meeting_settings ADD CONSTRAINT meeting_settings_agent_mode_check
    CHECK (agent_mode IN ('relevant', 'named', 'listen'));
ALTER TABLE meetings DROP CONSTRAINT meetings_agent_mode_check;
ALTER TABLE meetings ADD CONSTRAINT meetings_agent_mode_check
    CHECK (agent_mode IN ('relevant', 'named', 'listen'));
