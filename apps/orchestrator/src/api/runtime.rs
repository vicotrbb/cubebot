//! Machine credential boundary. It never consults human cookies or human session authority.
use crate::agents::{
    kubernetes::Error,
    runtime::{RuntimeAccess, RuntimeIdentity},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, Query, State, ws::WebSocketUpgrade},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use cubebot_protocol::runtime::{
    Bootstrap, Collaborate, Collaborated, Dream, DreamEnd, Memory, MemoryChange, REPORT_BYTES,
    RuntimeActor, RuntimeSession, SkillChange, SkillPackage,
};
use uuid::Uuid;

pub(super) fn router(identity: RuntimeIdentity) -> Router {
    Router::new()
        .route("/api/runtime/bootstrap", post(bootstrap))
        .route("/api/runtime/identity", get(inspect))
        .route("/api/runtime/channel", get(channel))
        .route("/api/runtime/memories", get(memories))
        .route(
            "/api/runtime/dreams/{run}",
            post(start_dream).put(finish_dream),
        )
        .route("/api/runtime/dreams/{run}/changes", post(commit_dream))
        .route("/api/runtime/skills/{name}", get(skill))
        .route("/api/runtime/turns/{turn}/messages", post(collaborate))
        // A package of at most 64 KiB, JSON-escaped.
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
async fn inspect(
    State(identity): State<RuntimeIdentity>,
    headers: HeaderMap,
) -> Result<Json<RuntimeActor>, Failure> {
    let token = bearer(&headers)?;
    let actor = identity
        .authenticate(token, RuntimeAccess::Active)
        .await
        .map_err(Failure::from)?;
    Ok(Json(actor.into()))
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
/// Searches only the authenticated agent's own memories.
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
/// Admits a dream run; 409 when the agent may not dream now.
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
/// Commits one change of a running run; 409 once it has ended.
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
/// Publishes one skill of a running run; 409 once it has ended.
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
/// The authenticated agent's installed revision of a skill; 404 when it is not installed.
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
/// Sends work from the agent's running turn to an agent sharing a room with it; a refusal is
/// 409 with the reason the model reads.
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
        Err(reason) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error": reason})),
        )
            .into_response(),
    })
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
    headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(Failure(StatusCode::UNAUTHORIZED))
}
struct Failure(StatusCode);
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
