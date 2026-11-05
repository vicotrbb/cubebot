use crate::{
    agents::AgentId,
    connections::Connections,
    identity::{
        Identity, IdentityError,
        access::{Capability, Scope},
        audit::Event,
    },
    speech::{AgentSpeech, SaveSpeech, Session, Speech, settings},
    work::Work,
};
use axum::{
    Json, Router,
    extract::{Path, State, ws::WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, put},
};
use serde_json::json;

const FRAME_LIMIT: usize = 64 * 1024;

#[derive(Clone)]
struct Api {
    identity: Identity,
    pool: sqlx::PgPool,
    speech: Speech,
    connections: Option<Connections>,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(read_settings, save_settings, save_agent))]
pub(super) struct Doc;

pub(super) fn router(
    pool: sqlx::PgPool,
    identity: Identity,
    speech: Speech,
    connections: Option<Connections>,
) -> Router {
    Router::new()
        .route("/api/agents/{id}/call", get(connect))
        .route("/api/agents/{id}/speech", put(save_agent))
        .route("/api/speech", get(read_settings).put(save_settings))
        .with_state(Api {
            identity,
            pool,
            speech,
            connections,
        })
}

#[tracing::instrument(skip_all, fields(agent = %agent.0))]
async fn connect(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, IdentityError> {
    let token = super::browser::credential(&api.identity, &headers)?.to_owned();
    // Browsers send cookies with cross-site WebSocket handshakes, and no CORS check stops them.
    let origin = headers.get("origin").and_then(|value| value.to_str().ok());
    if !headers.contains_key("authorization") && origin != api.identity.browser_origin.as_deref() {
        return Err(IdentityError::Forbidden);
    }
    let actor = api.identity.authenticate(&token).await?;
    let work = Work::new(api.pool.clone());
    work.may_call(&actor, agent).await?;
    let session = Session {
        speech: api.speech,
        work,
        identity: api.identity,
        pool: api.pool,
        agent,
        actor,
        token,
    };
    Ok(upgrade
        .max_message_size(FRAME_LIMIT)
        .on_upgrade(move |socket| session.run(socket)))
}

/// The installation's speech settings, without any key.
#[utoipa::path(get, path = "/api/speech", responses((status = 200, body = crate::speech::SpeechSettings)))]
async fn read_settings(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(settings::settings(&api.pool).await?),
    ))
}

/// Administrators only. A key is stored encrypted and never returned.
#[utoipa::path(put, path = "/api/speech", request_body = SaveSpeech, responses((status = 204)))]
async fn save_settings(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<SaveSpeech>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    settings::save(&api.pool, api.connections.as_ref(), &actor, &input).await?;
    let detail = json!({
        "stt_provider": input.stt_provider,
        "tts_provider": input.tts_provider,
        "voice": input.voice,
    });
    let event = Event::by(actor.id, "speech.configure").detail(detail);
    api.identity.audit(event).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Sets this agent's overrides; needs `agent.manage`.
#[utoipa::path(put, path = "/api/agents/{id}/speech", params(("id" = uuid::Uuid, Path)), request_body = AgentSpeech, responses((status = 204)))]
async fn save_agent(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
    Json(input): Json<AgentSpeech>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let scope = Scope::Agent { id: agent.0 };
    api.identity
        .authorize(&actor, scope, Capability::AgentManage)
        .await?;
    settings::save_agent(&api.pool, agent, &input).await?;
    let event = Event::by(actor.id, "agent.speech")
        .on(agent.0)
        .detail(json!(input));
    api.identity.audit(event).await;
    Ok(StatusCode::NO_CONTENT)
}
