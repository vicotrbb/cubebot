-- Each polled tool call finds its result by id; without this that scans the turn's events.
CREATE INDEX turn_events_tool_result ON turn_events (turn_id, (payload->>'id')) WHERE payload->>'kind'='tool_result';
