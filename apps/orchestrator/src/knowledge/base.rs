use crate::storage::Principal;
use serde::{Deserialize, Serialize};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

const HISTORY_WINDOW_SECONDS: f64 = 300.0;
const SEARCH_RESULTS: i64 = 30;

#[derive(Clone)]
pub struct KnowledgeBase {
    pool: PgPool,
}

#[derive(Debug)]
pub enum BaseError {
    Invalid(&'static str),
    NotFound,
    Stale(i64),
    Database(sqlx::Error),
}

impl From<sqlx::Error> for BaseError {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}

impl std::fmt::Display for BaseError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(reason) => formatter.write_str(reason),
            Self::NotFound => formatter.write_str("not found"),
            Self::Stale(revision) => write!(formatter, "the page is now at revision {revision}"),
            Self::Database(_) => formatter.write_str("knowledge base unavailable"),
        }
    }
}

impl std::error::Error for BaseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            _ => None,
        }
    }
}

/// A person's or agent's name on a page or version.
#[derive(Serialize, utoipa::ToSchema)]
pub struct PageAuthor {
    pub account: Option<Uuid>,
    pub agent: Option<Uuid>,
    pub name: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct PageSummary {
    pub id: Uuid,
    pub parent: Option<Uuid>,
    /// `page` or `folder`.
    pub kind: String,
    pub title: String,
    pub icon: Option<String>,
    pub revision: i64,
    pub updated_at: String,
    pub updated_by: PageAuthor,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Page {
    #[serde(flatten)]
    pub summary: PageSummary,
    /// Markdown.
    pub body: String,
    pub attachments: Vec<Attachment>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct Attachment {
    pub id: Uuid,
    pub page: Uuid,
    pub name: String,
    pub content_type: Option<String>,
    pub size: i64,
    pub created_at: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct SearchHit {
    pub id: Uuid,
    pub kind: String,
    pub title: String,
    pub icon: Option<String>,
    /// The matching text around the query.
    pub snippet: String,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct PageVersion {
    pub revision: i64,
    pub title: String,
    pub saved_at: String,
    pub by: PageAuthor,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct PageVersionBody {
    pub revision: i64,
    pub title: String,
    pub body: String,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct NewPage {
    /// Your id for this creation: a retry with it returns the page the first attempt made.
    pub request_id: Uuid,
    pub parent: Option<Uuid>,
    /// `page` or `folder`.
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub icon: Option<String>,
}

/// Replaces one exact passage, which must occur once, so it needs no revision to be safe.
#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Replace {
    pub old_text: String,
    pub new_text: String,
}

#[derive(Default, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PageChange {
    /// The revision the change was made from; required with `body`.
    #[serde(default)]
    pub expected_revision: Option<i64>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub replace: Option<Replace>,
    /// An empty icon removes it.
    #[serde(default)]
    pub icon: Option<String>,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct MovePage {
    /// The new parent; null is the top level.
    pub parent: Option<Uuid>,
    /// The sibling it goes after; null puts it first.
    pub after: Option<Uuid>,
}

pub struct KeptBlob {
    pub blob: Uuid,
    pub size: i64,
}

fn valid_title(title: &str) -> Result<(), BaseError> {
    if title.len() > 512 || title.chars().any(char::is_control) {
        return Err(BaseError::Invalid("a title is up to 512 bytes on one line"));
    }
    Ok(())
}

fn valid_body(body: &str) -> Result<(), BaseError> {
    if body.len() > 4 << 20 || body.contains('\0') {
        return Err(BaseError::Invalid("a page body is text up to 4 MiB"));
    }
    Ok(())
}

fn valid_icon(icon: &str) -> Result<(), BaseError> {
    if icon.len() > 32 || icon.chars().any(char::is_control) {
        return Err(BaseError::Invalid("an icon is a short symbol"));
    }
    Ok(())
}

fn replaced(text: &str, change: &Replace) -> Result<String, BaseError> {
    if change.old_text.is_empty() {
        return Err(BaseError::Invalid("the text to replace is empty"));
    }
    match text.matches(&change.old_text).count() {
        0 => Err(BaseError::Invalid(
            "the text to replace is not on the page; read it again",
        )),
        1 => Ok(text.replacen(&change.old_text, &change.new_text, 1)),
        _ => Err(BaseError::Invalid(
            "the text to replace occurs more than once; include more around it",
        )),
    }
}

#[derive(PartialEq)]
struct Stored {
    kind: String,
    title: String,
    body: String,
    icon: Option<String>,
    revision: i64,
}

impl PageChange {
    fn validate(&self) -> Result<(), BaseError> {
        if let Some(title) = &self.title {
            valid_title(title)?;
        }
        if let Some(icon) = &self.icon {
            valid_icon(icon)?;
        }
        if self.body.is_some() && self.expected_revision.is_none() {
            return Err(BaseError::Invalid("a new body names the revision it read"));
        }
        Ok(())
    }

    fn applied_to(&self, current: &Stored) -> Result<Stored, BaseError> {
        if self
            .expected_revision
            .is_some_and(|read| read != current.revision)
        {
            return Err(BaseError::Stale(current.revision));
        }
        let body = match (&self.body, &self.replace) {
            (Some(_), Some(_)) => {
                return Err(BaseError::Invalid(
                    "send a new body or a replacement, not both",
                ));
            }
            (Some(body), None) => body.clone(),
            (None, Some(replace)) => replaced(&current.body, replace)?,
            (None, None) => current.body.clone(),
        };
        valid_body(&body)?;
        if current.kind == "folder" && !body.is_empty() {
            return Err(BaseError::Invalid("a folder holds pages, not text"));
        }
        let icon = match self.icon.as_deref() {
            Some("") => None,
            Some(icon) => Some(icon.to_owned()),
            None => current.icon.clone(),
        };
        Ok(Stored {
            kind: current.kind.clone(),
            title: self.title.clone().unwrap_or_else(|| current.title.clone()),
            body,
            icon,
            revision: current.revision,
        })
    }
}

impl KnowledgeBase {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub async fn agent_allowed(&self, agent: Uuid) -> Result<bool, BaseError> {
        Ok(sqlx::query_scalar!(
            "
            SELECT knowledge_base
            FROM agents
            WHERE id=$1
            ",
            agent,
        )
        .fetch_optional(&self.pool)
        .await?
        .unwrap_or(false))
    }

    #[tracing::instrument(skip_all)]
    pub async fn tree(&self) -> Result<Vec<PageSummary>, BaseError> {
        self.summaries(false, None).await
    }

    #[tracing::instrument(skip_all)]
    pub async fn trash(&self) -> Result<Vec<PageSummary>, BaseError> {
        self.summaries(true, None).await
    }

    async fn summaries(
        &self,
        trashed: bool,
        only: Option<Uuid>,
    ) -> Result<Vec<PageSummary>, BaseError> {
        let rows = sqlx::query!(
            r#"
            SELECT p.id, p.parent_id, p.kind, p.title, p.icon, p.revision,
                (to_jsonb(p.updated_at) #>> '{}') AS "updated_at!",
                p.updated_by_account, p.updated_by_agent,
                COALESCE(h.login, a.name) AS "author!"
            FROM kb_pages p
            LEFT JOIN human_accounts h ON h.id = p.updated_by_account
            LEFT JOIN agents a ON a.id = p.updated_by_agent
            LEFT JOIN kb_pages up ON up.id = p.parent_id
            WHERE ($2::uuid IS NOT NULL AND p.id = $2 AND p.trashed_at IS NULL)
               OR ($2::uuid IS NULL AND (CASE WHEN $1
                    THEN p.trashed_at IS NOT NULL AND up.trashed_at IS DISTINCT FROM p.trashed_at
                    ELSE p.trashed_at IS NULL END))
            ORDER BY CASE WHEN $1 THEN p.trashed_at END DESC, p.position, p.title
            "#,
            trashed,
            only,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| PageSummary {
                id: row.id,
                parent: row.parent_id,
                kind: row.kind,
                title: row.title,
                icon: row.icon,
                revision: row.revision,
                updated_at: row.updated_at,
                updated_by: PageAuthor {
                    account: row.updated_by_account,
                    agent: row.updated_by_agent,
                    name: row.author,
                },
            })
            .collect())
    }

    #[tracing::instrument(skip_all)]
    pub async fn search(&self, query: &str) -> Result<Vec<SearchHit>, BaseError> {
        let query = query.trim();
        if query.is_empty() {
            return Ok(Vec::new());
        }
        let hits = sqlx::query_as!(
            SearchHit,
            r#"
            WITH q AS (SELECT websearch_to_tsquery('simple', $1) AS ts)
            SELECT p.id, p.kind, p.title, p.icon,
                ts_headline('simple', p.body, q.ts,
                    'StartSel="",StopSel="",MaxWords=24,MinWords=8,MaxFragments=1') AS "snippet!"
            FROM kb_pages p, q
            WHERE p.trashed_at IS NULL
              AND (p.search @@ q.ts OR p.title ILIKE '%' || $2 || '%')
            ORDER BY (p.title ILIKE '%' || $2 || '%') DESC, ts_rank(p.search, q.ts) DESC, p.title
            LIMIT $3
            "#,
            query,
            query
                .replace('\\', r"\\")
                .replace('%', r"\%")
                .replace('_', r"\_"),
            SEARCH_RESULTS,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(hits)
    }

    #[tracing::instrument(skip_all, fields(page = %id))]
    pub async fn page(&self, id: Uuid) -> Result<Page, BaseError> {
        let summary = self
            .summaries(false, Some(id))
            .await?
            .pop()
            .ok_or(BaseError::NotFound)?;
        let body = sqlx::query_scalar!(
            "
            SELECT body
            FROM kb_pages
            WHERE id=$1
            ",
            id,
        )
        .fetch_one(&self.pool)
        .await?;
        let attachments = sqlx::query_as!(
            Attachment,
            r#"
            SELECT x.id, x.page_id AS page, x.name, x.content_type, b.size,
                (to_jsonb(x.created_at) #>> '{}') AS "created_at!"
            FROM kb_attachments x JOIN blobs b ON b.id = x.blob_id
            WHERE x.page_id=$1
            ORDER BY x.created_at
            "#,
            id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(Page {
            summary,
            body,
            attachments,
        })
    }

    async fn live_parent(
        tx: &mut Transaction<'_, Postgres>,
        parent: Option<Uuid>,
    ) -> Result<(), BaseError> {
        let Some(parent) = parent else {
            return Ok(());
        };
        let alive = sqlx::query_scalar!(
            "
            SELECT trashed_at IS NULL AS \"alive!\"
            FROM kb_pages
            WHERE id=$1
            FOR SHARE
            ",
            parent,
        )
        .fetch_optional(&mut **tx)
        .await?;
        if alive != Some(true) {
            return Err(BaseError::Invalid(
                "the parent is not in the knowledge base",
            ));
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(request = %input.request_id))]
    pub async fn create(&self, who: Principal, input: &NewPage) -> Result<Page, BaseError> {
        valid_title(&input.title)?;
        valid_body(&input.body)?;
        if let Some(icon) = &input.icon {
            valid_icon(icon)?;
        }
        if !matches!(input.kind.as_str(), "page" | "folder") {
            return Err(BaseError::Invalid("kind is page or folder"));
        }
        if input.kind == "folder" && !input.body.is_empty() {
            return Err(BaseError::Invalid("a folder holds pages, not text"));
        }
        let (account, agent) = who.parts();
        let mut tx = self.pool.begin().await?;
        Self::live_parent(&mut tx, input.parent).await?;
        let id = sqlx::query_scalar!(
            "
            INSERT INTO kb_pages (parent_id, kind, title, body, icon, position, request_id,
                created_by_account, created_by_agent, updated_by_account, updated_by_agent)
            VALUES ($1, $2, $3, $4, $5,
                COALESCE((SELECT max(position) FROM kb_pages
                          WHERE parent_id IS NOT DISTINCT FROM $1), 0) + 1,
                $6, $7, $8, $7, $8)
            ON CONFLICT (request_id) DO UPDATE SET request_id = EXCLUDED.request_id
            RETURNING id
            ",
            input.parent,
            input.kind,
            input.title,
            input.body,
            input.icon,
            input.request_id,
            account,
            agent,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        self.page(id).await
    }

    #[tracing::instrument(skip_all, fields(page = %id))]
    pub async fn update(
        &self,
        who: Principal,
        id: Uuid,
        change: &PageChange,
    ) -> Result<Page, BaseError> {
        change.validate()?;
        let mut tx = self.pool.begin().await?;
        let current = sqlx::query_as!(
            Stored,
            "
            SELECT kind, title, body, icon, revision
            FROM kb_pages
            WHERE id=$1 AND trashed_at IS NULL
            FOR UPDATE
            ",
            id,
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(BaseError::NotFound)?;
        let next = change.applied_to(&current)?;
        if next != current {
            store(&mut tx, who, id, &current, &next).await?;
        }
        tx.commit().await?;
        self.page(id).await
    }

    #[tracing::instrument(skip_all, fields(page = %id))]
    pub async fn place(&self, id: Uuid, target: &MovePage) -> Result<(), BaseError> {
        let mut tx = self.pool.begin().await?;
        Self::live_parent(&mut tx, target.parent).await?;
        let cycle = sqlx::query_scalar!(
            r#"
            WITH RECURSIVE chain AS (
                SELECT id, parent_id FROM kb_pages WHERE id = $1
                UNION ALL
                SELECT p.id, p.parent_id FROM kb_pages p JOIN chain c ON p.id = c.parent_id
            )
            SELECT EXISTS (SELECT 1 FROM chain WHERE id = $2) AS "cycle!"
            "#,
            target.parent,
            id,
        )
        .fetch_one(&mut *tx)
        .await?;
        if cycle {
            return Err(BaseError::Invalid("a page cannot move inside itself"));
        }
        let moved = sqlx::query!(
            "
            WITH anchor AS (
                SELECT position FROM kb_pages
                WHERE id = $3 AND parent_id IS NOT DISTINCT FROM $2 AND id <> $1
            ), next AS (
                SELECT min(position) AS position FROM kb_pages
                WHERE parent_id IS NOT DISTINCT FROM $2 AND id <> $1
                  AND position > COALESCE((SELECT position FROM anchor), '-Infinity'::float8)
            )
            UPDATE kb_pages
            SET parent_id = $2,
                position = CASE
                    WHEN (SELECT position FROM anchor) IS NULL
                        THEN COALESCE((SELECT position FROM next), 1) - 1
                    WHEN (SELECT position FROM next) IS NULL
                        THEN (SELECT position FROM anchor) + 1
                    ELSE ((SELECT position FROM anchor) + (SELECT position FROM next)) / 2
                END
            WHERE id = $1 AND trashed_at IS NULL
            ",
            id,
            target.parent,
            target.after,
        )
        .execute(&mut *tx)
        .await?;
        if moved.rows_affected() == 0 {
            return Err(BaseError::NotFound);
        }
        tx.commit().await?;
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(page = %id))]
    pub async fn throw_away(&self, id: Uuid) -> Result<(), BaseError> {
        let trashed = sqlx::query!(
            "
            WITH RECURSIVE tree AS (
                SELECT id FROM kb_pages WHERE id = $1 AND trashed_at IS NULL
                UNION ALL
                SELECT p.id FROM kb_pages p JOIN tree t ON p.parent_id = t.id
                WHERE p.trashed_at IS NULL
            ), now AS (SELECT clock_timestamp() AS at)
            UPDATE kb_pages SET trashed_at = (SELECT at FROM now)
            WHERE id IN (SELECT id FROM tree)
            ",
            id,
        )
        .execute(&self.pool)
        .await?;
        if trashed.rows_affected() == 0 {
            return Err(BaseError::NotFound);
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(page = %id))]
    pub async fn restore(&self, id: Uuid) -> Result<(), BaseError> {
        let restored = sqlx::query!(
            "
            WITH RECURSIVE root AS (
                SELECT id, trashed_at FROM kb_pages WHERE id = $1 AND trashed_at IS NOT NULL
            ), tree AS (
                SELECT p.id FROM kb_pages p JOIN root r ON p.id = r.id
                UNION ALL
                SELECT p.id FROM kb_pages p JOIN tree t ON p.parent_id = t.id
                JOIN root r ON p.trashed_at = r.trashed_at
            )
            UPDATE kb_pages p
            SET trashed_at = NULL,
                parent_id = CASE
                    WHEN p.id = $1 AND EXISTS(
                        SELECT 1 FROM kb_pages up
                        WHERE up.id = p.parent_id AND up.trashed_at IS NOT NULL
                    ) THEN NULL
                    ELSE p.parent_id
                END
            WHERE p.id IN (SELECT id FROM tree)
            ",
            id,
        )
        .execute(&self.pool)
        .await?;
        if restored.rows_affected() == 0 {
            return Err(BaseError::NotFound);
        }
        Ok(())
    }

    #[tracing::instrument(skip_all, fields(page = %id))]
    pub async fn destroy(&self, id: Uuid) -> Result<Vec<Uuid>, BaseError> {
        let mut tx = self.pool.begin().await?;
        let blobs = sqlx::query_scalar!(
            "
            WITH RECURSIVE tree AS (
                SELECT id FROM kb_pages WHERE id = $1 AND trashed_at IS NOT NULL
                UNION ALL
                SELECT p.id FROM kb_pages p JOIN tree t ON p.parent_id = t.id
            )
            SELECT x.blob_id
            FROM kb_attachments x
            WHERE x.page_id IN (SELECT id FROM tree)
            ",
            id,
        )
        .fetch_all(&mut *tx)
        .await?;
        let gone = sqlx::query!(
            "
            DELETE FROM kb_pages
            WHERE id = $1 AND trashed_at IS NOT NULL
            ",
            id,
        )
        .execute(&mut *tx)
        .await?;
        if gone.rows_affected() == 0 {
            return Err(BaseError::NotFound);
        }
        tx.commit().await?;
        Ok(blobs)
    }

    #[tracing::instrument(skip_all, fields(page = %id))]
    pub async fn versions(&self, id: Uuid) -> Result<Vec<PageVersion>, BaseError> {
        let rows = sqlx::query!(
            r#"
            SELECT r.revision, r.title, (to_jsonb(r.saved_at) #>> '{}') AS "saved_at!",
                r.account_id, r.agent_id, COALESCE(h.login, a.name, 'Someone') AS "author!"
            FROM kb_revisions r
            LEFT JOIN human_accounts h ON h.id = r.account_id
            LEFT JOIN agents a ON a.id = r.agent_id
            WHERE r.page_id=$1
            ORDER BY r.revision DESC
            LIMIT 200
            "#,
            id,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|row| PageVersion {
                revision: row.revision,
                title: row.title,
                saved_at: row.saved_at,
                by: PageAuthor {
                    account: row.account_id,
                    agent: row.agent_id,
                    name: row.author,
                },
            })
            .collect())
    }

    #[tracing::instrument(skip_all, fields(page = %id))]
    pub async fn version(&self, id: Uuid, revision: i64) -> Result<PageVersionBody, BaseError> {
        sqlx::query_as!(
            PageVersionBody,
            "
            SELECT revision, title, body
            FROM kb_revisions
            WHERE page_id=$1 AND revision=$2
            ",
            id,
            revision,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(BaseError::NotFound)
    }

    pub async fn attachment_for(&self, request: Uuid) -> Result<Option<Attachment>, BaseError> {
        Ok(sqlx::query_as!(
            Attachment,
            r#"
            SELECT x.id, x.page_id AS page, x.name, x.content_type, b.size,
                (to_jsonb(x.created_at) #>> '{}') AS "created_at!"
            FROM kb_attachments x JOIN blobs b ON b.id = x.blob_id
            WHERE x.request_id=$1
            "#,
            request,
        )
        .fetch_optional(&self.pool)
        .await?)
    }

    pub async fn require_page(&self, page: Uuid) -> Result<(), BaseError> {
        sqlx::query_scalar!(
            "
            SELECT id
            FROM kb_pages
            WHERE id=$1 AND trashed_at IS NULL AND kind='page'
            ",
            page,
        )
        .fetch_optional(&self.pool)
        .await?
        .map(|_| ())
        .ok_or(BaseError::NotFound)
    }

    #[tracing::instrument(skip_all, fields(page = %page))]
    pub async fn attach(
        &self,
        who: Principal,
        page: Uuid,
        kept: &KeptBlob,
        name: &str,
        content_type: Option<&str>,
        request: Uuid,
    ) -> Result<Attachment, BaseError> {
        let (account, agent) = who.parts();
        let row = sqlx::query!(
            r#"
            INSERT INTO kb_attachments
                (id, page_id, blob_id, name, content_type, request_id, account_id, agent_id)
            VALUES (gen_random_uuid(), $1, $2, $3, $4, $5, $6, $7)
            RETURNING id, (to_jsonb(created_at) #>> '{}') AS "created_at!"
            "#,
            page,
            kept.blob,
            name,
            content_type,
            request,
            account,
            agent,
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(Attachment {
            id: row.id,
            page,
            name: name.to_owned(),
            content_type: content_type.map(Into::into),
            size: kept.size,
            created_at: row.created_at,
        })
    }

    pub async fn attachment(&self, id: Uuid) -> Result<(Attachment, Uuid), BaseError> {
        let row = sqlx::query!(
            r#"
            SELECT x.id, x.page_id, x.name, x.content_type, b.size, x.blob_id,
                (to_jsonb(x.created_at) #>> '{}') AS "created_at!"
            FROM kb_attachments x
            JOIN blobs b ON b.id = x.blob_id
            JOIN kb_pages p ON p.id = x.page_id AND p.trashed_at IS NULL
            WHERE x.id=$1
            "#,
            id,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(BaseError::NotFound)?;
        Ok((
            Attachment {
                id: row.id,
                page: row.page_id,
                name: row.name,
                content_type: row.content_type,
                size: row.size,
                created_at: row.created_at,
            },
            row.blob_id,
        ))
    }

    pub async fn detach(&self, id: Uuid) -> Result<Uuid, BaseError> {
        sqlx::query_scalar!(
            "
            DELETE FROM kb_attachments
            WHERE id=$1
            RETURNING blob_id
            ",
            id,
        )
        .fetch_optional(&self.pool)
        .await?
        .ok_or(BaseError::NotFound)
    }
}

async fn store(
    tx: &mut Transaction<'_, Postgres>,
    who: Principal,
    id: Uuid,
    current: &Stored,
    next: &Stored,
) -> Result<(), BaseError> {
    let revision = current.revision + 1;
    let (account, agent) = who.parts();
    sqlx::query!(
        "
        UPDATE kb_pages
        SET title=$2, body=$3, icon=$4, revision=$5, updated_by_account=$6,
            updated_by_agent=$7, updated_at=clock_timestamp()
        WHERE id=$1
        ",
        id,
        next.title,
        next.body,
        next.icon,
        revision,
        account,
        agent,
    )
    .execute(&mut **tx)
    .await?;
    if next.title != current.title || next.body != current.body {
        remember(tx, id, revision, &next.title, &next.body, who).await?;
    }
    Ok(())
}

async fn remember(
    tx: &mut Transaction<'_, Postgres>,
    page: Uuid,
    revision: i64,
    title: &str,
    body: &str,
    who: Principal,
) -> Result<(), BaseError> {
    if merged(tx, page, revision, title, body, who).await? {
        return Ok(());
    }
    let (account, agent) = who.parts();
    sqlx::query!(
        "
        INSERT INTO kb_revisions (page_id, revision, title, body, account_id, agent_id)
        VALUES ($1, $2, $3, $4, $5, $6)
        ",
        page,
        revision,
        title,
        body,
        account,
        agent,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

async fn merged(
    tx: &mut Transaction<'_, Postgres>,
    page: Uuid,
    revision: i64,
    title: &str,
    body: &str,
    who: Principal,
) -> Result<bool, BaseError> {
    let (account, agent) = who.parts();
    let merged = sqlx::query!(
        "
        UPDATE kb_revisions
        SET revision=$2, title=$3, body=$4, saved_at=clock_timestamp()
        WHERE page_id=$1
          AND revision=(SELECT max(revision) FROM kb_revisions WHERE page_id=$1)
          AND account_id IS NOT DISTINCT FROM $5
          AND agent_id IS NOT DISTINCT FROM $6
          AND started_at > clock_timestamp() - make_interval(secs => $7)
        ",
        page,
        revision,
        title,
        body,
        account,
        agent,
        HISTORY_WINDOW_SECONDS,
    )
    .execute(&mut **tx)
    .await?;
    Ok(merged.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_replacement_needs_exactly_one_occurrence() {
        let change = |old: &str| Replace {
            old_text: old.into(),
            new_text: "X".into(),
        };
        assert_eq!(replaced("a b c", &change("b")).unwrap(), "a X c");
        assert!(replaced("a b c", &change("z")).is_err());
        assert!(replaced("a a", &change("a")).is_err());
        assert!(replaced("a", &change("")).is_err());
    }
}
