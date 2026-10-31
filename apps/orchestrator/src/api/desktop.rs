//! A person's view of an agent's desktop (ADRs 0039 and 0085): a WebSocket proxied to the
//! sandbox's host channel. Frames go to the page as the sandbox encoded them; the orchestrator
//! never decodes or keeps them. Watching takes `browser.view` and taking control
//! `browser.control`, and the session rechecks the viewer's credential and access while it runs.
use crate::{
    agents::{
        AgentId, Agents,
        sandbox::{Desktop, ExecChannel},
    },
    identity::{
        Actor, Identity, IdentityError,
        access::{Capability, Scope},
    },
};
use axum::{
    Extension, Router,
    extract::{
        Path, State,
        ws::{CloseFrame, Message, WebSocket, WebSocketUpgrade},
    },
    http::HeaderMap,
    response::Response,
    routing::get,
};
use cubebot_protocol::desktop::{self, Action, HostRequest, LINE_LIMIT, Mode};
use serde::Deserialize;
use serde_json::json;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use uuid::Uuid;

/// The shortest time between frame requests; the next is asked only once the last is sent, so a
/// slow page slows its own frames.
const FRAME_INTERVAL: Duration = Duration::from_millis(50);
const RECHECK: Duration = Duration::from_secs(30);
/// Bounds one sandbox response: a connection's first frame holds the whole screen.
const RESPONSE_LIMIT: u64 = 32 << 20;

/// Why a session ends; the Display text is the close reason the page shows.
#[derive(Debug)]
enum Closing {
    InvalidMessage(serde_json::Error),
    Unavailable(Box<dyn std::error::Error + Send + Sync>),
    /// The sandbox closed the channel or sent a line without its end.
    Truncated,
    // IdentityError has no std::error::Error impl to carry as a source.
    NotRecorded,
    AccessEnded,
    Disconnected(axum::Error),
}

impl std::fmt::Display for Closing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidMessage(_) => "invalid desktop message",
            Self::Unavailable(_) | Self::Truncated => "the desktop is unavailable",
            Self::NotRecorded => "the desktop session could not be recorded",
            Self::AccessEnded => "access to this desktop ended",
            Self::Disconnected(_) => "the viewer disconnected",
        })
    }
}

impl std::error::Error for Closing {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidMessage(error) => Some(error),
            Self::Unavailable(error) => Some(error.as_ref()),
            Self::Disconnected(error) => Some(error),
            Self::Truncated | Self::NotRecorded | Self::AccessEnded => None,
        }
    }
}

fn unavailable(error: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Closing {
    Closing::Unavailable(error.into())
}

#[derive(Clone)]
struct DesktopApi {
    identity: Identity,
    agents: Agents,
}

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/agents/{id}/desktop", get(connect))
        .with_state(DesktopApi {
            identity,
            agents: Agents::new(pool),
        })
}

/// From the page. `t` comes back as `ack` on the answer, so the page times the round trip.
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum Viewer {
    Take { t: f64 },
    HandBack { t: f64 },
    Input { t: f64, action: Action },
}

#[tracing::instrument(skip_all, fields(agent = %agent.0))]
async fn connect(
    State(api): State<DesktopApi>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
    desktop: Option<Extension<Desktop>>,
    upgrade: WebSocketUpgrade,
) -> Result<Response, IdentityError> {
    let token = super::browser::credential(&api.identity, &headers)?.to_owned();
    // Browsers send cookies with cross-site WebSocket handshakes, and no CORS check stops them.
    let origin = headers.get("origin").and_then(|value| value.to_str().ok());
    if !headers.contains_key("authorization") && origin != api.identity.browser_origin.as_deref() {
        return Err(IdentityError::Forbidden);
    }
    let actor = api.identity.authenticate(&token).await?;
    allowed(&api.identity, &actor, agent, Capability::BrowserView).await?;
    let Some(Extension(desktop)) = desktop else {
        return Err(IdentityError::NotFound);
    };
    let channel = desktop
        .open(agent)
        .await
        .map_err(|_| IdentityError::Conflict)?;
    let session = api.agents.start_desktop(agent, actor.id).await?;
    let viewer = Session {
        api,
        desktop,
        agent,
        token,
        actor,
        session,
        holding: false,
        channel: Channel::new(channel),
    };
    Ok(upgrade
        .max_message_size(LINE_LIMIT)
        .on_upgrade(move |socket| viewer.run(socket)))
}

async fn allowed(
    identity: &Identity,
    actor: &Actor,
    agent: AgentId,
    capability: Capability,
) -> Result<(), IdentityError> {
    let scope = Scope::Agent { id: agent.0 };
    identity.authorize(actor, scope, capability).await
}

struct Session {
    api: DesktopApi,
    desktop: Desktop,
    agent: AgentId,
    token: String,
    actor: Actor,
    session: Uuid,
    /// Whether this viewer took control and has not handed it back.
    holding: bool,
    channel: Channel,
}

impl Session {
    #[tracing::instrument(skip_all, fields(agent = %self.agent.0, session = %self.session))]
    async fn run(mut self, mut socket: WebSocket) {
        *self.viewers().entry(self.agent).or_default() += 1;
        // Without an account this never takes control from a person who holds it.
        let _ = self.mode(Mode::Watch, None).await;
        if let Err(reason) = self.serve(&mut socket).await {
            let close = CloseFrame {
                code: 1011,
                reason: reason.to_string().into(),
            };
            let _ = socket.send(Message::Close(Some(close))).await;
        }
        self.leave().await;
    }

    fn viewers(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<AgentId, usize>> {
        self.desktop
            .viewers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Hands back control this viewer still holds, and the desktop to the agent alone when no
    /// one else is watching.
    async fn leave(&mut self) {
        let remaining = {
            let mut viewers = self.viewers();
            let count = viewers.entry(self.agent).or_default();
            *count = count.saturating_sub(1);
            let remaining = *count;
            if remaining == 0 {
                viewers.remove(&self.agent);
            }
            remaining
        };
        let mode = if remaining == 0 {
            Mode::Agent
        } else {
            Mode::Watch
        };
        let account = self.holding.then_some(self.actor.id);
        if account.is_some() || remaining == 0 {
            let _ = self.mode(mode, account).await;
        }
        if let Err(error) = self.api.agents.end_desktop(self.session).await {
            tracing::warn!(?error, session = %self.session, "desktop session end not recorded");
        }
    }

    async fn serve(&mut self, socket: &mut WebSocket) -> Result<(), Closing> {
        let mut frames = tokio::time::interval(FRAME_INTERVAL);
        frames.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut recheck = tokio::time::interval(RECHECK);
        recheck.reset();
        loop {
            tokio::select! {
                message = socket.recv() => match message {
                    Some(Ok(Message::Text(text))) => {
                        let answer = self.answer(&text).await?;
                        send(socket, answer).await?;
                    }
                    Some(Ok(Message::Close(_)) | Err(_)) | None => return Ok(()),
                    Some(Ok(_)) => {}
                },
                _ = frames.tick() => {
                    let frame = self.channel.request(&HostRequest::Frame).await?;
                    send(socket, frame).await?;
                }
                _ = recheck.tick() => self.recheck().await?,
            }
        }
    }

    async fn answer(&mut self, text: &str) -> Result<String, Closing> {
        let message = serde_json::from_str(text).map_err(Closing::InvalidMessage)?;
        let (t, line) = match message {
            Viewer::Take { t } => (t, self.take().await?),
            Viewer::HandBack { t } if self.holding => {
                (t, self.mode(Mode::Watch, Some(self.actor.id)).await?)
            }
            Viewer::HandBack { t } => (t, json!({"error":"you do not have control"}).to_string()),
            Viewer::Input { t, action } => {
                let account = self.actor.id;
                let input = HostRequest::Input { account, action };
                (t, self.channel.request(&input).await?)
            }
        };
        let mut answer: serde_json::Value = serde_json::from_str(&line).map_err(unavailable)?;
        answer["ack"] = json!(t);
        Ok(answer.to_string())
    }

    async fn take(&mut self) -> Result<String, Closing> {
        if allowed(
            &self.api.identity,
            &self.actor,
            self.agent,
            Capability::BrowserControl,
        )
        .await
        .is_err()
        {
            let refusal = "you do not have permission to control this desktop";
            return Ok(json!({ "error": refusal }).to_string());
        }
        self.mode(Mode::Human, Some(self.actor.id)).await
    }

    /// Records the changes the viewer makes to their own control.
    async fn mode(&mut self, mode: Mode, account: Option<Uuid>) -> Result<String, Closing> {
        let line = self
            .channel
            .request(&HostRequest::Mode { mode, account })
            .await?;
        let changed = serde_json::from_str::<desktop::Response>(&line)
            .is_ok_and(|response| response.error.is_none());
        if changed && account.is_some() {
            self.holding = mode == Mode::Human;
            self.api
                .agents
                .record_desktop_mode(self.session, mode)
                .await
                .map_err(|_| Closing::NotRecorded)?;
        }
        Ok(line)
    }

    async fn recheck(&mut self) -> Result<(), Closing> {
        let identity = &self.api.identity;
        self.actor = identity
            .authenticate(&self.token)
            .await
            .map_err(|_| Closing::AccessEnded)?;
        allowed(identity, &self.actor, self.agent, Capability::BrowserView)
            .await
            .map_err(|_| Closing::AccessEnded)?;
        if self.holding {
            allowed(
                identity,
                &self.actor,
                self.agent,
                Capability::BrowserControl,
            )
            .await
            .map_err(|_| Closing::AccessEnded)?;
        }
        Ok(())
    }
}

async fn send(socket: &mut WebSocket, text: String) -> Result<(), Closing> {
    socket
        .send(Message::Text(text.into()))
        .await
        .map_err(Closing::Disconnected)
}

/// One request line to the sandbox's arbiter and its response line, in order.
struct Channel {
    input: Box<dyn AsyncWrite + Send + Unpin>,
    output: BufReader<Box<dyn AsyncRead + Send + Unpin>>,
    _session: Box<dyn Send>,
}

impl Channel {
    fn new(channel: ExecChannel) -> Self {
        Self {
            input: channel.stdin,
            output: BufReader::new(channel.stdout),
            _session: channel._session,
        }
    }

    async fn request(&mut self, request: &HostRequest) -> Result<String, Closing> {
        let mut line = serde_json::to_vec(request).map_err(unavailable)?;
        line.push(b'\n');
        self.input.write_all(&line).await.map_err(unavailable)?;
        self.input.flush().await.map_err(unavailable)?;
        let mut response = Vec::new();
        let mut bounded = (&mut self.output).take(RESPONSE_LIMIT);
        bounded
            .read_until(b'\n', &mut response)
            .await
            .map_err(unavailable)?;
        if response.pop() != Some(b'\n') {
            return Err(Closing::Truncated);
        }
        String::from_utf8(response).map_err(|error| unavailable(error.utf8_error()))
    }
}
