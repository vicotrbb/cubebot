ALTER TABLE creation_requests ADD COLUMN reported_in uuid REFERENCES chat_turns(id);
CREATE INDEX creation_requests_reported ON creation_requests(reported_in) WHERE reported_in IS NOT NULL;

ALTER TABLE creation_requests DROP CONSTRAINT creation_requests_kind_check;
ALTER TABLE creation_requests ADD CHECK (kind IN ('agent', 'room', 'team', 'change'));
