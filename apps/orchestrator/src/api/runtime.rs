//! Machine credential boundary. It never consults human cookies or human session authority.
use super::refused;
use crate::agents::{
    kubernetes::Error,
    runtime::{RuntimeAccess, RuntimeIdentity},
};
use crate::storage::Principal;
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State, ws::WebSocketUpgrade},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cubebot_protocol::runtime::{
    Bootstrap, Broadcast, Broadcasted, Collaborate, Collaborated, Dream, DreamEnd, FileShare,
    Memory, MemoryChange, ProviderKey, REPORT_BYTES, RequestLogin, RoutineAnswer, RoutineCall,
    RuntimeSession, SkillChange, SkillPackage, VaultFill, VaultItem, VaultPut,
};
use uuid::Uuid;

pub(super) fn router(identity: RuntimeIdentity) -> Router {
    Router::new()
        .route("/api/runtime/bootstrap", post(bootstrap))
        .route("/api/runtime/channel", get(channel))
        .route("/api/runtime/memories", get(memories))
        .route(
            "/api/runtime/dreams/{run}",
            post(start_dream).put(finish_dream),
        )
        .route("/api/runtime/dreams/{run}/changes", post(commit_dream))
        .route("/api/runtime/skills/{name}", get(skill))
        .route("/api/runtime/wallpaper", get(wallpaper))
        .route("/api/runtime/provider", get(provider))
        .route("/api/runtime/turns/{turn}/messages", post(collaborate))
        .route("/api/runtime/turns/{turn}/logins", post(request_login))
        .route("/api/runtime/turns/{turn}/broadcasts", post(broadcast))
        .route("/api/runtime/turns/{turn}/routines", post(routine))
        .route("/api/runtime/turns/{turn}/attachments", post(attach))
        .route(
            "/api/runtime/turns/{turn}/skills",
            post(skill_tool).layer(DefaultBodyLimit::max(160 * 1024)),
        )
        .route("/api/runtime/vault", get(vault).post(vault_put))
        .route("/api/runtime/vault/fills", post(vault_fill))
        .route("/api/runtime/files", get(files).post(upload_file))
        .route("/api/runtime/files/{file}/content", get(file_content))
        .route("/api/runtime/files/{file}/shares", post(share_file))
        .route(
            "/api/runtime/dreams/{run}/skills",
            post(commit_dream_skill).layer(DefaultBodyLimit::max(160 * 1024)),
        )
        .layer(DefaultBodyLimit::max(20 * 1024))
        .layer(axum::middleware::map_response(no_store))
        .with_state(identity)
}
async fn bootstrap(
    State(identity): State<RuntimeIdentity>,
    input: Result<Json<Bootstrap>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<RuntimeSession>, Failure> {
    let Json(input) = input.map_err(|error| Failure(error.status()))?;
    let session = identity
        .bootstrap(&input.token, input.instance)
        .await
        .map_err(Failure::from)?;
    Ok(Json(session.into()))
}
/// Authenticates before upgrading; the channel rechecks the credential while it runs.
async fn channel(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> Result<Response, Failure> {
    let token = bearer(&headers)?.to_owned();
    identity
        .authenticate(&token, RuntimeAccess::Recovery)
        .await
        .map_err(Failure::from)?;
    Ok(upgrade
        .max_message_size(2 * cubebot_protocol::runtime::EVENT_BYTES)
        .on_upgrade(move |socket| identity.serve(token, socket)))
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct MemoryQuery {
    query: String,
}
async fn memories(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Query(input): Query<MemoryQuery>,
) -> Result<Json<Vec<Memory>>, Failure> {
    let actor = identity
        .authenticate(bearer(&headers)?, RuntimeAccess::Active)
        .await
        .map_err(Failure::from)?;
    if input.query.trim().is_empty() || input.query.len() > 1024 || input.query.contains('\0') {
        return Err(Failure(StatusCode::BAD_REQUEST));
    }
    let found = crate::knowledge::Knowledge::new(identity.pool.clone())
        .search_for_agent(actor.agent, &input.query)
        .await
        .map_err(|error| {
            tracing::warn!(?error, "memory search failed");
            Failure(StatusCode::SERVICE_UNAVAILABLE)
        })?;
    Ok(Json(
        found
            .into_iter()
            .map(|memory| Memory {
                id: memory.id,
                revision: memory.revision,
                content: memory.content,
            })
            .collect(),
    ))
}
async fn start_dream(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Path(run): Path<Uuid>,
) -> Result<Json<Dream>, Failure> {
    let actor = active(&identity, &headers).await?;
    crate::knowledge::start_dream(&identity.pool, identity.connections.as_ref(), &actor, run)
        .await
        .map_err(unavailable)?
        .map(Json)
        .ok_or(Failure(StatusCode::CONFLICT))
}
async fn commit_dream(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Path(run): Path<Uuid>,
    Json(change): Json<MemoryChange>,
) -> Result<StatusCode, Failure> {
    let actor = active(&identity, &headers).await?;
    if !crate::knowledge::valid_change(&change) {
        return Err(Failure(StatusCode::BAD_REQUEST));
    }
    let open = crate::knowledge::commit_dream(&identity.pool, &actor, run, &change)
        .await
        .map_err(unavailable)?;
    Ok(if open {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CONFLICT
    })
}
async fn commit_dream_skill(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Path(run): Path<Uuid>,
    Json(change): Json<SkillChange>,
) -> Result<StatusCode, Failure> {
    let actor = active(&identity, &headers).await?;
    let metadata =
        crate::knowledge::package(&change.files).ok_or(Failure(StatusCode::BAD_REQUEST))?;
    let open =
        crate::knowledge::commit_dream_skill(&identity.pool, &actor, run, &change, &metadata)
            .await
            .map_err(unavailable)?;
    Ok(if open {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CONFLICT
    })
}
async fn skill(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Path(name): Path<String>,
) -> Result<Json<SkillPackage>, Failure> {
    let actor = active(&identity, &headers).await?;
    crate::knowledge::skill_packages(&identity.pool, actor.agent, Some(&name), 1)
        .await
        .map_err(unavailable)?
        .pop()
        .map(Json)
        .ok_or(Failure(StatusCode::NOT_FOUND))
}
async fn wallpaper(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
) -> Result<Response, Failure> {
    let actor = active(&identity, &headers).await?;
    let image = crate::agents::wallpaper(&identity.pool, actor.agent)
        .await
        .map_err(unavailable)?
        .ok_or(Failure(StatusCode::NOT_FOUND))?;
    let content_type = crate::agents::content_type(&image).unwrap_or("application/octet-stream");
    Ok(([(header::CONTENT_TYPE, content_type)], image).into_response())
}
async fn collaborate(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Path(turn): Path<Uuid>,
    Json(input): Json<Collaborate>,
) -> Result<Response, Failure> {
    let actor = active(&identity, &headers).await?;
    let created = crate::work::Work::new(identity.pool.clone())
        .collaborate(&actor, turn, &input)
        .await
        .map_err(Failure::from)?;
    Ok(match created {
        Ok(turn) => Json(Collaborated { turn }).into_response(),
        Err(reason) => refused(&reason),
    })
}
async fn request_login(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Path(turn): Path<Uuid>,
    Json(input): Json<RequestLogin>,
) -> Result<Response, Failure> {
    let actor = active(&identity, &headers).await?;
    let recorded = crate::work::Work::new(identity.pool.clone())
        .request_login(&actor, turn, &input)
        .await
        .map_err(Failure::from)?;
    Ok(match recorded {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(reason) => refused(&reason),
    })
}
async fn broadcast(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Path(turn): Path<Uuid>,
    Json(input): Json<Broadcast>,
) -> Result<Response, Failure> {
    let actor = active(&identity, &headers).await?;
    let sent = crate::work::Work::new(identity.pool.clone())
        .agent_broadcast(&actor, turn, &input)
        .await
        .map_err(Failure::from)?;
    Ok(match sent {
        Ok(agents) => Json(Broadcasted { agents }).into_response(),
        Err(reason) => refused(&reason),
    })
}
async fn routine(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Path(turn): Path<Uuid>,
    Json(input): Json<RoutineCall>,
) -> Result<Response, Failure> {
    let actor = active(&identity, &headers).await?;
    let answer = crate::work::Work::new(identity.pool.clone())
        .routine_tool(&actor, turn, &input)
        .await
        .map_err(|error| {
            tracing::warn!(?error, "runtime routine request failed");
            Failure(StatusCode::SERVICE_UNAVAILABLE)
        })?;
    Ok(match answer {
        Ok(output) => Json(RoutineAnswer { output }).into_response(),
        Err(reason) => refused(&reason),
    })
}
async fn skill_tool(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Path(turn): Path<Uuid>,
    Json(input): Json<RoutineCall>,
) -> Result<Response, Failure> {
    let actor = active(&identity, &headers).await?;
    let answer = crate::knowledge::Knowledge::new(identity.pool.clone())
        .skill_tool(&actor, turn, &input)
        .await
        .map_err(|error| {
            tracing::warn!(?error, "runtime skill request failed");
            Failure(StatusCode::SERVICE_UNAVAILABLE)
        })?;
    Ok(match answer {
        Ok(output) => Json(RoutineAnswer { output }).into_response(),
        Err(reason) => refused(&reason),
    })
}

async fn attach(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Path(turn): Path<Uuid>,
    Json(input): Json<cubebot_protocol::runtime::AttachFile>,
) -> Result<Response, Failure> {
    let actor = active(&identity, &headers).await?;
    let answer = crate::work::Work::new(identity.pool.clone())
        .attach_file(&actor, turn, input.file)
        .await
        .map_err(|error| {
            tracing::warn!(?error, "runtime attachment request failed");
            Failure(StatusCode::SERVICE_UNAVAILABLE)
        })?;
    Ok(match answer {
        Ok(()) => StatusCode::NO_CONTENT.into_response(),
        Err(reason) => refused(&reason),
    })
}

fn vault_of(identity: &RuntimeIdentity) -> Result<&crate::connections::Connections, Failure> {
    identity
        .connections
        .as_ref()
        .ok_or(Failure(StatusCode::SERVICE_UNAVAILABLE))
}
fn vault_unavailable(error: crate::identity::IdentityError) -> Failure {
    tracing::warn!(?error, "runtime vault request failed");
    Failure(StatusCode::SERVICE_UNAVAILABLE)
}
/// The agent's current provider material: a subscription's access token, renewed when due.
async fn provider(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
) -> Result<Json<ProviderKey>, Failure> {
    let actor = active(&identity, &headers).await?;
    let connections = identity
        .connections
        .as_ref()
        .ok_or(Failure(StatusCode::SERVICE_UNAVAILABLE))?;
    let inference = connections.current(actor.agent).await.map_err(|error| {
        tracing::warn!(?error, "runtime provider material unavailable");
        Failure(StatusCode::SERVICE_UNAVAILABLE)
    })?;
    Ok(Json(ProviderKey {
        provider: inference.provider,
        api_key: inference.api_key,
        api_key_expires_at: inference.api_key_expires_at,
    }))
}
async fn vault(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
) -> Result<Json<Vec<VaultItem>>, Failure> {
    let actor = active(&identity, &headers).await?;
    let items = vault_of(&identity)?.runtime_vault(actor.agent).await;
    Ok(Json(items.map_err(vault_unavailable)?))
}
async fn vault_put(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Json(input): Json<VaultPut>,
) -> Result<Response, Failure> {
    let actor = active(&identity, &headers).await?;
    let saved = vault_of(&identity)?.vault_put(actor.agent, &input).await;
    Ok(match saved.map_err(vault_unavailable)? {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(reason) => refused(reason),
    })
}
/// The text never comes back.
#[tracing::instrument(skip_all, fields(item = %input.item))]
async fn vault_fill(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    desktop: Option<axum::Extension<crate::agents::sandbox::Desktop>>,
    Json(input): Json<VaultFill>,
) -> Result<Response, Failure> {
    let actor = active(&identity, &headers).await?;
    let text = vault_of(&identity)?
        .vault_fill_text(actor.agent, None, input.item, input.field)
        .await
        .map_err(vault_unavailable)?;
    let Some(text) = text else {
        return Ok(refused("no vault item with this id is available to you"));
    };
    let Some(axum::Extension(desktop)) = desktop else {
        return Ok(refused("the desktop is unavailable"));
    };
    match super::desktop::fill(&desktop, actor.agent, input.item, text, None).await {
        Ok(Ok(())) => Ok(StatusCode::NO_CONTENT.into_response()),
        Ok(Err(reason)) => Ok(refused(&reason)),
        Err(error) => {
            tracing::warn!(?error, "vault fill did not reach the desktop");
            Ok(refused("the desktop is unavailable"))
        }
    }
}
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct FilesQuery {
    parent: Option<Uuid>,
    #[serde(default)]
    shared: bool,
}
type Storage = Option<axum::Extension<crate::storage::Storage>>;
async fn files(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    storage: Storage,
    Query(query): Query<FilesQuery>,
) -> Result<Response, Failure> {
    let who = Principal::Agent(active(&identity, &headers).await?.agent);
    let listed = match super::files::configured(storage) {
        Ok(storage) if query.shared => storage.shared(who).await,
        Ok(storage) => storage.list(who, query.parent).await,
        Err(error) => Err(error),
    };
    Ok(listed.map_or_else(super::files::refused, |entries| {
        Json(entries).into_response()
    }))
}
async fn upload_file(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    storage: Storage,
    Query(query): Query<super::files::UploadQuery>,
    body: axum::body::Body,
) -> Result<Response, Failure> {
    let who = Principal::Agent(active(&identity, &headers).await?.agent);
    let stored = match super::files::configured(storage) {
        Ok(storage) => super::files::store(&storage, who, query, &headers, body).await,
        Err(error) => Err(error),
    };
    Ok(stored.unwrap_or_else(super::files::refused))
}
async fn file_content(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    storage: Storage,
    Path(file): Path<Uuid>,
) -> Result<Response, Failure> {
    let who = Principal::Agent(active(&identity, &headers).await?.agent);
    let opened = match super::files::configured(storage) {
        Ok(storage) => storage.open(who, file).await,
        Err(error) => Err(error),
    };
    Ok(match opened {
        Ok((entry, bytes)) => super::files::download(entry, bytes, &headers).await,
        Err(error) => super::files::refused(error),
    })
}
async fn share_file(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    storage: Storage,
    Path(file): Path<Uuid>,
    Json(share): Json<FileShare>,
) -> Result<Response, Failure> {
    let who = Principal::Agent(active(&identity, &headers).await?.agent);
    let shared = match super::files::configured(storage) {
        Ok(storage) => storage.share(who, file, &[share], false).await,
        Err(error) => Err(error),
    };
    Ok(shared.map_or_else(super::files::refused, |shares| Json(shares).into_response()))
}
async fn finish_dream(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
    Path(run): Path<Uuid>,
    Json(end): Json<DreamEnd>,
) -> Result<StatusCode, Failure> {
    let actor = active(&identity, &headers).await?;
    if end.report.len() > REPORT_BYTES || end.report.contains('\0') {
        return Err(Failure(StatusCode::BAD_REQUEST));
    }
    let ended = crate::knowledge::finish_dream(&identity.pool, &actor, run, &end)
        .await
        .map_err(unavailable)?;
    Ok(if ended {
        StatusCode::NO_CONTENT
    } else {
        StatusCode::CONFLICT
    })
}
async fn active(
    identity: &RuntimeIdentity,
    headers: &HeaderMap,
) -> Result<crate::agents::runtime::RuntimeActor, Failure> {
    identity
        .authenticate(bearer(headers)?, RuntimeAccess::Active)
        .await
        .map_err(Failure::from)
}
fn unavailable(error: sqlx::Error) -> Failure {
    tracing::warn!(?error, "runtime knowledge request failed");
    Failure(StatusCode::SERVICE_UNAVAILABLE)
}
fn bearer(headers: &HeaderMap) -> Result<&str, Failure> {
    super::bearer(headers).ok_or(Failure(StatusCode::UNAUTHORIZED))
}
pub(super) struct Failure(pub(super) StatusCode);
impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self(match error {
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Conflict => StatusCode::CONFLICT,
            Error::Busy => StatusCode::TOO_MANY_REQUESTS,
            Error::InvalidConfiguration | Error::Configuration(_) => StatusCode::BAD_REQUEST,
            _ => StatusCode::SERVICE_UNAVAILABLE,
        })
    }
}
impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        (
            self.0,
            Json(serde_json::json!({"error":"runtime_identity_unavailable"})),
        )
            .into_response()
    }
}

async fn no_store(mut response: Response) -> Response {
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    if response.status() == StatusCode::TOO_MANY_REQUESTS {
        response.headers_mut().insert(
            header::RETRY_AFTER,
            axum::http::HeaderValue::from_static("1"),
        );
    }
    response
}
