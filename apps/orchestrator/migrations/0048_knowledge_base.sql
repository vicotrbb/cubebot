-- The knowledge base: one wiki of pages and folders for the whole installation. Every person can
-- read and write it, and every agent while its capability is on, which is the default. A page's
-- body is Markdown: agents read and edit it as text, and the console edits the same text as
-- blocks. Attachments are blobs on the storage volume.
ALTER TABLE agents ADD COLUMN knowledge_base boolean NOT NULL DEFAULT true;
ALTER TABLE agent_definition_revisions ADD COLUMN knowledge_base boolean NOT NULL DEFAULT true;

CREATE TABLE kb_pages (
    id uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    parent_id uuid REFERENCES kb_pages(id) ON DELETE CASCADE,
    kind text NOT NULL CHECK (kind IN ('page', 'folder')),
    title text NOT NULL CHECK (octet_length(title) <= 512),
    icon text CHECK (octet_length(icon) BETWEEN 1 AND 32),
    body text NOT NULL DEFAULT '' CHECK (octet_length(body) <= 4194304),
    CHECK (kind = 'page' OR body = ''),
    -- Every save, so a stale write is refused instead of overwriting a newer one.
    revision bigint NOT NULL DEFAULT 1 CHECK (revision > 0),
    position double precision NOT NULL,
    -- The caller's id for the creation, so a resent call finds the page it made.
    request_id uuid UNIQUE,
    created_by_account uuid REFERENCES human_accounts(id),
    created_by_agent uuid REFERENCES agents(id),
    updated_by_account uuid REFERENCES human_accounts(id),
    updated_by_agent uuid REFERENCES agents(id),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    updated_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    -- In the trash since then, with everything under it, which shares the timestamp.
    trashed_at timestamptz,
    search tsvector GENERATED ALWAYS AS (
        setweight(to_tsvector('simple', title), 'A') || setweight(to_tsvector('simple', body), 'B')
    ) STORED,
    CHECK (num_nonnulls(created_by_account, created_by_agent) = 1),
    CHECK (num_nonnulls(updated_by_account, updated_by_agent) = 1)
);
CREATE INDEX kb_pages_parent ON kb_pages (parent_id, position);
CREATE INDEX kb_pages_search ON kb_pages USING gin (search);

-- Earlier versions of a page. One author's saves within a few minutes share a row, so an
-- autosaving editor leaves a readable history.
CREATE TABLE kb_revisions (
    page_id uuid NOT NULL REFERENCES kb_pages(id) ON DELETE CASCADE,
    revision bigint NOT NULL,
    title text NOT NULL,
    body text NOT NULL,
    account_id uuid REFERENCES human_accounts(id),
    agent_id uuid REFERENCES agents(id),
    started_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    saved_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY (page_id, revision)
);

CREATE TABLE kb_attachments (
    id uuid PRIMARY KEY,
    page_id uuid NOT NULL REFERENCES kb_pages(id) ON DELETE CASCADE,
    blob_id uuid NOT NULL REFERENCES blobs(id),
    name text NOT NULL,
    content_type text,
    request_id uuid UNIQUE,
    account_id uuid REFERENCES human_accounts(id),
    agent_id uuid REFERENCES agents(id),
    created_at timestamptz NOT NULL DEFAULT clock_timestamp()
);
CREATE INDEX kb_attachments_page ON kb_attachments (page_id);
