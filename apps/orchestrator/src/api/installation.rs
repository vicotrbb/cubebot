use crate::{
    agents::{Agent, AgentId, Agents},
    identity::{Identity, IdentityError, audit::Event},
    operations::settings::{Settings, deployment_name},
    storage::Storage,
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
    pool: sqlx::PgPool,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(installation, events, settings, save_settings, about))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    crate::operations::about::mark_start();
    Router::new()
        .route("/api/installation/about", get(about))
        .route("/api/installation", get(installation))
        .route(
            "/api/installation/settings",
            get(settings).put(save_settings),
        )
        .route("/api/events", get(events))
        .with_state(Api {
            identity,
            agents: Agents::new(pool.clone()),
            work: Work::new(pool.clone()),
            pool,
        })
}

#[derive(Debug, Clone)]
pub struct Installation {
    pub controller: &'static str,
    pub terminal_enabled: bool,
    pub computer_use_enabled: bool,
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
    /// The administrator's name for it, else `CUBEBOT_INSTALLATION_NAME`, else "Cubebot".
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
            name: Settings::load(&api.pool).await?.name(),
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

#[derive(Serialize, utoipa::ToSchema)]
struct SettingsView {
    #[serde(flatten)]
    settings: Settings,
    /// The deployment's name, used while `name` is empty.
    default_name: String,
    /// The shared-file volume; absent when the installation has none.
    storage: Option<StorageView>,
}

#[derive(Serialize, utoipa::ToSchema)]
struct StorageView {
    /// The volume's size, when the filesystem reports it.
    capacity_bytes: Option<u64>,
    /// The quota in force: the setting or the deployment's, within 90% of the volume.
    quota_bytes: u64,
    default_quota_bytes: u64,
    /// The per-file limit in force; absent when uploads are unlimited.
    upload_limit_bytes: Option<u64>,
    default_upload_limit_bytes: Option<u64>,
    used_bytes: u64,
}

fn view(settings: Settings, storage: Option<&Storage>) -> SettingsView {
    SettingsView {
        settings,
        default_name: deployment_name(),
        storage: storage.map(|storage| {
            let usage = storage.usage();
            StorageView {
                capacity_bytes: usage.capacity,
                quota_bytes: usage.quota,
                default_quota_bytes: usage.defaults.quota,
                upload_limit_bytes: usage.upload,
                default_upload_limit_bytes: usage.defaults.upload,
                used_bytes: usage.used,
            }
        }),
    }
}

#[utoipa::path(get, path = "/api/installation/about", operation_id = "installation_about", responses((status = 200, body = crate::operations::about::About)))]
async fn about(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    administrator(&api, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(crate::operations::about::about(&api.pool).await?),
    ))
}

async fn administrator(api: &Api, headers: &HeaderMap) -> Result<Uuid, IdentityError> {
    let actor = super::identity::actor(&api.identity, headers).await?;
    if !actor.administrator {
        return Err(IdentityError::Forbidden);
    }
    Ok(actor.id)
}

/// Administrators only.
#[utoipa::path(get, path = "/api/installation/settings", operation_id = "installation_settings", responses((status = 200, body = SettingsView)))]
async fn settings(
    State(api): State<Api>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
) -> Result<impl IntoResponse, IdentityError> {
    administrator(&api, &headers).await?;
    let settings = Settings::load(&api.pool).await?;
    let storage = storage.map(|Extension(storage)| storage);
    Ok((
        [("cache-control", "no-store")],
        Json(view(settings, storage.as_ref())),
    ))
}

/// Administrators only; applies at once. 409 when `revision` is not the current one.
#[utoipa::path(put, path = "/api/installation/settings", operation_id = "save_installation_settings", request_body = Settings, responses((status = 200, body = SettingsView), (status = 409)))]
async fn save_settings(
    State(api): State<Api>,
    headers: HeaderMap,
    storage: Option<Extension<Storage>>,
    Json(input): Json<Settings>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = administrator(&api, &headers).await?;
    let saved = Settings::save(&api.pool, &input).await?;
    let storage = storage.map(|Extension(storage)| storage);
    if let Some(storage) = &storage {
        let (quota, upload) = saved.limits();
        storage.limit(quota, upload);
    }
    let detail = serde_json::json!({
        "name": saved.name,
        "storage_quota_bytes": saved.storage_quota_bytes,
        "upload_limit_bytes": saved.upload_limit_bytes,
    });
    api.identity
        .audit(Event::by(actor, "installation.settings").detail(detail))
        .await;
    Ok((
        [("cache-control", "no-store")],
        Json(view(saved, storage.as_ref())),
    ))
}

/// Follows every agent and room the caller may see: `agent` frames carry an `Agent` summary
/// whenever its activity, current tool, desktop, pending login or last turn changed, and every visible
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
        broadcast: None,
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
        super::STREAM_HEADERS,
        Sse::new(frames).keep_alive(KeepAlive::default()),
    ))
}

type AgentFrame = (String, bool, bool, i64, Option<String>, Option<String>);

struct Follow {
    api: Api,
    headers: HeaderMap,
    agents: HashMap<AgentId, AgentFrame>,
    rooms: HashMap<Uuid, i64>,
    broadcast: Option<i64>,
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
                agent.current_tool.clone(),
                agent.current_target.clone(),
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
        let broadcast = self.api.work.broadcast_sequence(&actor).await?;
        if let Some(last_sequence) = broadcast.filter(|_| broadcast != self.broadcast) {
            frames.push(frame(
                "broadcast",
                &serde_json::json!({ "last_sequence": last_sequence }),
            ));
        }
        self.broadcast = broadcast;
        Ok(frames)
    }
}
