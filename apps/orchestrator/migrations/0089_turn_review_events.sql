ALTER TABLE turn_events DROP CONSTRAINT turn_events_payload_check;
ALTER TABLE turn_events ADD CHECK (payload->>'kind' IN ('text_delta','tool_call','tool_result','usage','review','summary','steered','done','error'));
