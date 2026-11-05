//! The knowledge base for people and, under `/api/runtime/kb`, for agents whose capability is on.
//! One set of handlers: [`Gate`] says who is calling and they share everything after that.
use super::files::{self, UploadQuery};
use crate::{
    agents::runtime::{RuntimeAccess, RuntimeIdentity},
    identity::{Identity, IdentityError, audit::Event},
    knowledge::{
        Attachment, BaseError, KnowledgeBase, MovePage, NewPage, Page, PageChange, PageSummary,
        PageVersion, PageVersionBody, SearchHit,
    },
    storage::{Principal, Storage, StorageError},
};
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cubebot_protocol::runtime::{FileAccess, FileEntry};
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

#[derive(Clone)]
enum Gate {
    People(Identity),
    Agents(Box<RuntimeIdentity>),
}

#[derive(Clone)]
struct Api {
    gate: Gate,
    base: KnowledgeBase,
}

enum Failure {
    Identity(IdentityError),
    Base(BaseError),
    Storage(StorageError),
    /// A bearer that is not an active runtime.
    Runtime(StatusCode),
    /// The agent's knowledge base capability is off.
    Disabled,
}

impl From<IdentityError> for Failure {
    fn from(error: IdentityError) -> Self {
        Self::Identity(error)
    }
}
impl From<BaseError> for Failure {
    fn from(error: BaseError) -> Self {
        Self::Base(error)
    }
}
impl From<StorageError> for Failure {
    fn from(error: StorageError) -> Self {
        Self::Storage(error)
    }
}

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let refusal = |status: StatusCode, reason: String| {
            let body = Json(json!({ "error": reason }));
            (status, [(header::CACHE_CONTROL, "no-store")], body).into_response()
        };
        match self {
            Self::Identity(error) => error.into_response(),
            Self::Storage(error) => files::refused(error),
            Self::Runtime(status) => status.into_response(),
            Self::Disabled => refusal(
                StatusCode::FORBIDDEN,
                "this agent's knowledge base capability is off".into(),
            ),
            Self::Base(BaseError::Stale(revision)) => {
                let body = Json(json!({
                    "error": format!("the page changed; it is now at revision {revision}"),
                    "revision": revision,
                }));
                (
                    StatusCode::CONFLICT,
                    [(header::CACHE_CONTROL, "no-store")],
                    body,
                )
                    .into_response()
            }
            Self::Base(BaseError::NotFound) => refusal(StatusCode::NOT_FOUND, "not_found".into()),
            Self::Base(BaseError::Invalid(reason)) => {
                refusal(StatusCode::BAD_REQUEST, reason.into())
            }
            Self::Base(BaseError::Database(error)) => {
                tracing::warn!(?error, "knowledge base request failed");
                refusal(StatusCode::SERVICE_UNAVAILABLE, "unavailable".into())
            }
        }
    }
}

impl Api {
    async fn caller(&self, headers: &HeaderMap) -> Result<Principal, Failure> {
        match &self.gate {
            Gate::People(identity) => {
                let actor = super::identity::actor(identity, headers).await?;
                Ok(Principal::Person(actor.id))
            }
            Gate::Agents(identity) => {
                let bearer =
                    super::bearer(headers).ok_or(Failure::Runtime(StatusCode::UNAUTHORIZED))?;
                let actor = identity
                    .authenticate(bearer, RuntimeAccess::Active)
                    .await
                    .map_err(|error| Failure::Runtime(super::runtime::Failure::from(error).0))?;
                if !self.base.agent_allowed(actor.agent.0).await? {
                    return Err(Failure::Disabled);
                }
                Ok(Principal::Agent(actor.agent))
            }
        }
    }

    /// Records what people do; an agent's work is on the page as its author.
    async fn audit(&self, who: Principal, kind: &'static str, page: Uuid) {
        if let (Gate::People(identity), Principal::Person(id)) = (&self.gate, who) {
            let event = Event::by(id, kind).detail(json!({ "page": page }));
            identity.audit(event).await;
        }
    }
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    tree, create, search, trash, page, update, throw_away, place, restore, destroy, versions,
    version, attach, content, detach
))]
pub(super) struct Doc;

pub(super) fn router(identity: Identity, pool: sqlx::PgPool) -> Router {
    routes("/api/kb", Gate::People(identity), pool)
}

pub(super) fn runtime_router(identity: RuntimeIdentity, pool: sqlx::PgPool) -> Router {
    routes("/api/runtime/kb", Gate::Agents(Box::new(identity)), pool)
}

fn routes(prefix: &str, gate: Gate, pool: sqlx::PgPool) -> Router {
    let path = |rest: &str| format!("{prefix}{rest}");
    Router::new()
        .route(&path("/pages"), get(tree).post(create))
        .route(&path("/search"), get(search))
        .route(&path("/trash"), get(trash))
        .route(
            &path("/pages/{id}"),
            get(page).patch(update).delete(throw_away),
        )
        .route(&path("/pages/{id}/place"), post(place))
        .route(&path("/pages/{id}/restore"), post(restore))
        .route(&path("/pages/{id}/purge"), post(destroy))
        .route(&path("/pages/{id}/versions"), get(versions))
        .route(&path("/pages/{id}/versions/{revision}"), get(version))
        .route(&path("/pages/{id}/attachments"), post(attach))
        .route(
            &path("/attachments/{id}"),
            get(content).delete(detach),
        )
        // A page body of at most 4 MiB, JSON-escaped; an attachment's body is a stream.
        .layer(DefaultBodyLimit::max(10 * 1024 * 1024))
        .with_state(Api {
            gate,
            base: KnowledgeBase::new(pool),
        })
}

/// Every page and folder outside the trash, in reading order, without bodies.
#[utoipa::path(get, path = "/api/kb/pages", operation_id = "kb_tree", responses((status = 200, body = Vec<PageSummary>)))]
async fn tree(State(api): State<Api>, headers: HeaderMap) -> Result<Response, Failure> {
    api.caller(&headers).await?;
    Ok(fresh(api.base.tree().await?))
}

#[utoipa::path(post, path = "/api/kb/pages", operation_id = "kb_create", request_body = NewPage, responses((status = 201, body = Page)))]
async fn create(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<NewPage>,
) -> Result<Response, Failure> {
    let who = api.caller(&headers).await?;
    let page = api.base.create(who, &input).await?;
    api.audit(who, "kb.create", page.summary.id).await;
    Ok((StatusCode::CREATED, Json(page)).into_response())
}

#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
struct SearchQuery {
    q: String,
}

#[utoipa::path(get, path = "/api/kb/search", operation_id = "kb_search", params(SearchQuery), responses((status = 200, body = Vec<SearchHit>)))]
async fn search(
    State(api): State<Api>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
) -> Result<Response, Failure> {
    api.caller(&headers).await?;
    Ok(fresh(api.base.search(&query.q).await?))
}

/// What was thrown away on its own, newest first.
#[utoipa::path(get, path = "/api/kb/trash", operation_id = "kb_trash", responses((status = 200, body = Vec<PageSummary>)))]
async fn trash(State(api): State<Api>, headers: HeaderMap) -> Result<Response, Failure> {
    api.caller(&headers).await?;
    Ok(fresh(api.base.trash().await?))
}

#[utoipa::path(get, path = "/api/kb/pages/{id}", operation_id = "kb_page", params(("id" = Uuid, Path)), responses((status = 200, body = Page)))]
async fn page(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, Failure> {
    api.caller(&headers).await?;
    Ok(fresh(api.base.page(id).await?))
}

/// 409 with the current revision when the page moved on from `expected_revision`.
#[utoipa::path(patch, path = "/api/kb/pages/{id}", operation_id = "kb_update", params(("id" = Uuid, Path)), request_body = PageChange, responses((status = 200, body = Page), (status = 409)))]
async fn update(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(change): Json<PageChange>,
) -> Result<Response, Failure> {
    let who = api.caller(&headers).await?;
    Ok(fresh(api.base.update(who, id, &change).await?))
}

/// Into the trash, with everything under it.
#[utoipa::path(delete, path = "/api/kb/pages/{id}", operation_id = "kb_throw_away", params(("id" = Uuid, Path)), responses((status = 204)))]
async fn throw_away(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Failure> {
    let who = api.caller(&headers).await?;
    api.base.throw_away(id).await?;
    api.audit(who, "kb.trash", id).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/kb/pages/{id}/place", operation_id = "kb_place", params(("id" = Uuid, Path)), request_body = MovePage, responses((status = 204)))]
async fn place(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(target): Json<MovePage>,
) -> Result<StatusCode, Failure> {
    api.caller(&headers).await?;
    api.base.place(id, &target).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/kb/pages/{id}/restore", operation_id = "kb_restore", params(("id" = Uuid, Path)), responses((status = 204)))]
async fn restore(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Failure> {
    let who = api.caller(&headers).await?;
    api.base.restore(id).await?;
    api.audit(who, "kb.restore", id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes something in the trash for good. People only: an agent can only throw things away.
#[utoipa::path(post, path = "/api/kb/pages/{id}/purge", operation_id = "kb_purge", params(("id" = Uuid, Path)), responses((status = 204)))]
async fn destroy(
    State(api): State<Api>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Failure> {
    let who = api.caller(&headers).await?;
    if matches!(who, Principal::Agent(_)) {
        return Err(StorageError::Forbidden.into());
    }
    let blobs = api.base.destroy(id).await?;
    if let Some(Extension(storage)) = storage {
        storage.discard(&blobs).await?;
    }
    api.audit(who, "kb.purge", id).await;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/kb/pages/{id}/versions", operation_id = "kb_versions", params(("id" = Uuid, Path)), responses((status = 200, body = Vec<PageVersion>)))]
async fn versions(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, Failure> {
    api.caller(&headers).await?;
    Ok(fresh(api.base.versions(id).await?))
}

#[utoipa::path(get, path = "/api/kb/pages/{id}/versions/{revision}", operation_id = "kb_version", params(("id" = Uuid, Path), ("revision" = i64, Path)), responses((status = 200, body = PageVersionBody)))]
async fn version(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((id, revision)): Path<(Uuid, i64)>,
) -> Result<Response, Failure> {
    api.caller(&headers).await?;
    Ok(fresh(api.base.version(id, revision).await?))
}

/// The body is the attachment's bytes and its content type the attachment's.
#[utoipa::path(post, path = "/api/kb/pages/{id}/attachments", operation_id = "kb_attach", params(("id" = Uuid, Path), UploadQuery), request_body(content = Vec<u8>, content_type = "application/octet-stream"), responses((status = 201, body = Attachment), (status = 200, body = Attachment)))]
async fn attach(
    State(api): State<Api>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
    Path(page): Path<Uuid>,
    Query(query): Query<UploadQuery>,
    body: Body,
) -> Result<Response, Failure> {
    let who = api.caller(&headers).await?;
    let storage = files::configured(storage)?;
    if let Some(attachment) = api.base.attachment_for(query.request_id).await? {
        return Ok(Json(attachment).into_response());
    }
    api.base.require_page(page).await?;
    crate::storage::named(&query.name)?;
    let kept = storage.keep(body.into_data_stream()).await?;
    let content_type = crate::storage::media_type(
        headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok()),
    );
    let attached = api
        .base
        .attach(
            who,
            page,
            &kept,
            &query.name,
            content_type.as_deref(),
            query.request_id,
        )
        .await;
    match attached {
        Ok(attachment) => Ok((StatusCode::CREATED, Json(attachment)).into_response()),
        Err(error) => {
            // A row never came to name these bytes.
            storage.discard(&[kept.blob]).await?;
            Err(error.into())
        }
    }
}

/// The bytes, with the same headers as a shared file: an `<img>` shows them, a tab downloads them.
#[utoipa::path(get, path = "/api/kb/attachments/{id}", operation_id = "kb_attachment", params(("id" = Uuid, Path)), responses((status = 200, body = Vec<u8>, content_type = "application/octet-stream"), (status = 206), (status = 416)))]
async fn content(
    State(api): State<Api>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
    Path(id): Path<Uuid>,
) -> Result<Response, Failure> {
    api.caller(&headers).await?;
    let storage = files::configured(storage)?;
    let (attachment, blob) = api.base.attachment(id).await?;
    let bytes = storage.read(blob, attachment.size).await?;
    let entry = FileEntry {
        id: attachment.id,
        parent: None,
        name: attachment.name,
        folder: false,
        size: Some(attachment.size),
        content_type: attachment.content_type,
        sha256: None,
        owner_user: None,
        owner_agent: None,
        access: FileAccess::Read,
        updated_at: attachment.created_at,
    };
    Ok(files::download(entry, bytes, &headers).await)
}

#[utoipa::path(delete, path = "/api/kb/attachments/{id}", operation_id = "kb_detach", params(("id" = Uuid, Path)), responses((status = 204)))]
async fn detach(
    State(api): State<Api>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Failure> {
    api.caller(&headers).await?;
    let storage = files::configured(storage)?;
    let blob = api.base.detach(id).await?;
    storage.discard(&[blob]).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A JSON body that is never cached, since every save changes it.
fn fresh(body: impl serde::Serialize) -> Response {
    ([(header::CACHE_CONTROL, "no-store")], Json(body)).into_response()
}
