CREATE TABLE push_key (
    id boolean PRIMARY KEY DEFAULT true CHECK (id),
    pkcs8 bytea NOT NULL CHECK (octet_length(pkcs8) BETWEEN 1 AND 512),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);

CREATE TABLE push_subscriptions (
    endpoint text PRIMARY KEY CHECK (endpoint ~ '^https://' AND octet_length(endpoint) <= 1024),
    account_id uuid NOT NULL REFERENCES human_accounts(id) ON DELETE CASCADE,
    p256dh bytea NOT NULL CHECK (octet_length(p256dh) = 65),
    auth bytea NOT NULL CHECK (octet_length(auth) = 16),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX push_subscriptions_account ON push_subscriptions (account_id, created_at);

ALTER TABLE human_accounts ADD COLUMN pushed_through timestamptz;
