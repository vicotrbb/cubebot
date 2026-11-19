ALTER TABLE vault_items DROP CONSTRAINT vault_items_kind_check;
ALTER TABLE vault_items ADD CHECK (kind IN ('login', 'secret', 'card'));
ALTER TABLE vault_items
    ADD COLUMN card_brand text CHECK (octet_length(card_brand) BETWEEN 1 AND 32),
    ADD COLUMN card_last4 text CHECK (card_last4 ~ '^[0-9]{4}$'),
    ADD COLUMN card_expires text CHECK (card_expires ~ '^(0[1-9]|1[0-2])/[0-9]{2}$'),
    ADD CHECK ((kind = 'card') = (card_brand IS NOT NULL AND card_last4 IS NOT NULL AND card_expires IS NOT NULL));

CREATE TABLE card_fills (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    agent_id uuid NOT NULL REFERENCES agents(id),
    item_id uuid NOT NULL REFERENCES vault_items(id) ON DELETE CASCADE,
    turn_id uuid NOT NULL REFERENCES chat_turns(id),
    call_id uuid NOT NULL UNIQUE,
    card_name text NOT NULL CHECK (octet_length(card_name) BETWEEN 1 AND 255),
    domain text NOT NULL CHECK (octet_length(domain) BETWEEN 1 AND 255),
    amount text NOT NULL CHECK (octet_length(amount) BETWEEN 1 AND 100),
    status text NOT NULL DEFAULT 'pending' CHECK (status IN ('pending', 'approved', 'declined')),
    settled_by uuid REFERENCES human_accounts(id),
    settled_at timestamptz,
    reason text CHECK (octet_length(reason) <= 2048),
    approved_until timestamptz,
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK ((status = 'pending') = (settled_at IS NULL)),
    CHECK ((status = 'approved') = (approved_until IS NOT NULL))
);
CREATE UNIQUE INDEX card_fills_one_pending ON card_fills(agent_id, item_id) WHERE status = 'pending';
CREATE INDEX card_fills_turn ON card_fills(turn_id);
