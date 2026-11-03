-- MCP servers (docs/adr/agents.md#local-and-remote-mcp-server-support) are installation configuration that administrators manage
-- in the console. Access follows docs/adr/identity.md#installation-wide-connection-access-by-default: every agent by default, or the agents
-- granted explicitly. A stdio server's environment and an HTTP server's headers may hold credentials,
-- so their values are one Credentials record bound to the server's id; only their names are readable.
CREATE TABLE mcp_servers (
    id uuid PRIMARY KEY,
    name text NOT NULL UNIQUE CHECK (name ~ '^[A-Za-z0-9-]{1,32}$'),
    transport text NOT NULL CHECK (transport IN ('stdio', 'http')),
    command text CHECK (octet_length(command) BETWEEN 1 AND 4096),
    args jsonb NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(args) = 'array'),
    url text CHECK (octet_length(url) BETWEEN 1 AND 2048),
    secret_names jsonb NOT NULL DEFAULT '[]' CHECK (jsonb_typeof(secret_names) = 'array'),
    ciphertext bytea NOT NULL,
    access jsonb NOT NULL DEFAULT '{"mode":"all_agents"}',
    created_by uuid NOT NULL REFERENCES human_accounts(id),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    CHECK ((transport = 'stdio') = (command IS NOT NULL)),
    CHECK ((transport = 'http') = (url IS NOT NULL))
);
