//! What the console knows about the installation, and its one stream per browser session.
use crate::{
    agents::{Agent, AgentId, Agents},
    identity::{Identity, IdentityError},
    work::Work,
};
use axum::{
    Extension, Json, Router,
    extract::State,
    http::HeaderMap,
    response::{
        IntoResponse,
        sse::{self, KeepAlive, Sse},
    },
    routing::get,
};
use futures_util::{StreamExt, stream};
use serde::Serialize;
use std::{collections::HashMap, convert::Infallible, time::Duration};
use uuid::Uuid;

fn frame(event: &str, data: &impl Serialize) -> sse::Event {
    sse::Event::default()
        .event(event)
        .data(serde_json::to_string(data).unwrap_or_default())
}

#[derive(Clone)]
struct Api {
    identity: Identity,
    agents: Agents,
    work: Work,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(installation, events))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/installation", get(installation))
        .route("/api/events", get(events))
        .with_state(Api {
            identity,
            agents: Agents::new(pool.clone()),
            work: Work::new(pool),
        })
}

/// The sandbox flags of the active controller; the controller attaches it to the router.
#[derive(Debug, Clone)]
pub struct Installation {
    /// `kubernetes`, `docker` or `database_only`.
    pub controller: &'static str,
    pub terminal_enabled: bool,
    pub computer_use_enabled: bool,
    /// None without a controller that runs agents.
    pub runtime: Option<crate::agents::sandbox::Runtime>,
}

impl Default for Installation {
    fn default() -> Self {
        Self {
            controller: "database_only",
            terminal_enabled: false,
            computer_use_enabled: false,
            runtime: None,
        }
    }
}

#[derive(Serialize, utoipa::ToSchema)]
struct InstallationSummary {
    /// `CUBEBOT_INSTALLATION_NAME`, or "Cubebot".
    name: String,
    terminal_enabled: bool,
    computer_use_enabled: bool,
    /// `kubernetes`, `docker` or `database_only`.
    controller: &'static str,
    /// The declared sandbox runtime: `kvm` (a virtual machine) or `container`; none without a
    /// controller that runs agents.
    sandbox_runtime: Option<&'static str>,
    /// The RuntimeClass or Docker runtime a `container` runtime names.
    sandbox_runtime_class: Option<String>,
}

#[utoipa::path(get, path = "/api/installation", responses((status = 200, body = InstallationSummary)))]
async fn installation(
    State(api): State<Api>,
    headers: HeaderMap,
    flags: Option<Extension<Installation>>,
) -> Result<impl IntoResponse, IdentityError> {
    super::identity::actor(&api.identity, &headers).await?;
    let flags = flags.map_or_else(Installation::default, |Extension(flags)| flags);
    Ok((
        [("cache-control", "no-store")],
        Json(InstallationSummary {
            name: std::env::var("CUBEBOT_INSTALLATION_NAME")
                .ok()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| "Cubebot".into()),
            terminal_enabled: flags.terminal_enabled,
            computer_use_enabled: flags.computer_use_enabled,
            controller: flags.controller,
            sandbox_runtime: flags
                .runtime
                .as_ref()
                .map(crate::agents::sandbox::Runtime::kind),
            sandbox_runtime_class: flags
                .runtime
                .as_ref()
                .and_then(|runtime| runtime.class().map(str::to_owned)),
        }),
    ))
}

/// Follows every agent and room the caller may see: `agent` frames carry an `Agent` summary
/// whenever its activity, desktop, pending login or last turn changed, and every visible
/// agent once at the start; `room` frames carry `{id, last_sequence}` when a room's newest
/// message changed. Polled once a second.
#[utoipa::path(get, path = "/api/events", operation_id = "installation_events", responses((status = 200, content_type = "text/event-stream", body = Agent)))]
async fn events(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let mut follow = Follow {
        api,
        headers,
        agents: HashMap::new(),
        rooms: HashMap::new(),
    };
    // The first poll runs before responding so a refusal is an HTTP status, not a closed stream.
    let first = follow.poll().await?;
    let live = stream::unfold(follow, |mut follow| async move {
        // shortcut: two queries a second per open console; use LISTEN/NOTIFY if measured
        // database load requires it.
        tokio::time::sleep(Duration::from_secs(1)).await;
        let frames = follow.poll().await.ok()?;
        Some((frames, follow))
    });
    let frames = stream::iter([super::opening(first)])
        .chain(live)
        .flat_map(stream::iter)
        .map(Ok::<_, Infallible>);
    Ok((
        [("cache-control", "no-store")],
        Sse::new(frames).keep_alive(KeepAlive::default()),
    ))
}

struct Follow {
    api: Api,
    headers: HeaderMap,
    /// What each agent last looked like: activity, desktop, pending login, last sequence.
    agents: HashMap<AgentId, (String, bool, bool, i64)>,
    rooms: HashMap<Uuid, i64>,
}

impl Follow {
    /// Reauthenticates each poll, so signing out or losing access ends the stream.
    async fn poll(&mut self) -> Result<Vec<sse::Event>, IdentityError> {
        let actor = super::identity::actor(&self.api.identity, &self.headers).await?;
        let mut frames = Vec::new();
        for agent in self.api.agents.visible(&actor).await? {
            let state = (
                agent.activity.clone(),
                agent.desktop_active,
                agent.pending_login,
                agent.last_sequence,
            );
            if self.agents.insert(agent.id, state.clone()) != Some(state) {
                frames.push(frame("agent", &agent));
            }
        }
        for room in self.api.work.room_sequences(&actor).await? {
            if self.rooms.insert(room.id, room.last_sequence) != Some(room.last_sequence) {
                frames.push(frame("room", &room));
            }
        }
        Ok(frames)
    }
}
