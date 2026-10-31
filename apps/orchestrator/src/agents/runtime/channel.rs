//! One WebSocket per sandbox: turns, cancellation, steering, drain, revocation and wallpaper
//! changes go down; turn events come up. Several turns can be active, one per lane; the ledger
//! decides admission and stays authoritative, the socket is only a delivery path.
use super::{Error, RuntimeAccess, RuntimeDisposition, RuntimeIdentity};
use crate::work::Work;
use axum::extract::ws::{Message, WebSocket};
use cubebot_protocol::runtime::{Delivery, Report, TurnEvent};
use std::time::Duration;
use uuid::Uuid;

#[derive(Default)]
struct Channel {
    /// The agent accepts turns; it stops at a drain.
    pulling: bool,
    /// The agent heard there is no work since it last received a turn.
    idle: bool,
    active: std::collections::HashSet<Uuid>,
    /// Active turns already sent `cancel`; they stay active until the agent reports their end.
    cancelled: std::collections::HashSet<Uuid>,
    drain: Option<Uuid>,
    /// Steering sent on this connection; a reconnect resends what is still unincorporated.
    steered: std::collections::HashSet<Uuid>,
    /// The wallpaper revision this connection last saw; the agent fetches it when it connects.
    wallpaper: Option<i64>,
}

impl Channel {
    /// Events reveal the active turn after a reconnect, so a cancel still reaches it.
    fn observe(&mut self, turn: Uuid, live: bool, event: &TurnEvent) {
        let finished = matches!(event, TurnEvent::Done { .. } | TurnEvent::Error { .. });
        if finished {
            self.active.remove(&turn);
            self.cancelled.remove(&turn);
        } else if live {
            self.active.insert(turn);
        }
    }
}

impl RuntimeIdentity {
    /// One span per connection.
    #[tracing::instrument(skip_all)]
    pub async fn serve(self, token: String, mut socket: WebSocket) {
        let mut channel = Channel::default();
        // shortcut: each connection polls the ledger every second; use LISTEN/NOTIFY if
        // measured delivery latency or database load requires it.
        let mut tick = tokio::time::interval(Duration::from_secs(1));
        loop {
            let deliveries = tokio::select! {
                incoming = socket.recv() => match incoming {
                    Some(Ok(Message::Text(text))) => match serde_json::from_str(&text) {
                        Ok(report) => self.receive(&token, &mut channel, report).await,
                        Err(error) => Err(Error::Configuration(error.into())),
                    },
                    Some(Ok(Message::Ping(_) | Message::Pong(_))) => Ok(Vec::new()),
                    _ => return,
                },
                _ = tick.tick() => self.poll(&token, &mut channel).await,
            };
            if !deliver(&mut socket, deliveries).await {
                return;
            }
        }
    }

    async fn receive(
        &self,
        token: &str,
        channel: &mut Channel,
        report: Report,
    ) -> Result<Vec<Delivery>, Error> {
        match report {
            Report::Pull => {
                channel.pulling = channel.drain.is_none();
                self.poll(token, channel).await
            }
            Report::Event {
                turn,
                sequence,
                event,
            } => {
                let actor = self.authenticate(token, RuntimeAccess::Recovery).await?;
                let live = Work::new(self.pool.clone())
                    .record(&actor, turn, sequence, &event)
                    .await?;
                channel.observe(turn, live, &event);
                Ok(vec![Delivery::Ack { turn, sequence }])
            }
            Report::Drained { request_id } => {
                self.acknowledge_drain(token, request_id).await?;
                // The agent may exit; stopping the sandbox remains the hard stop.
                Ok(vec![Delivery::Revoked])
            }
        }
    }

    async fn poll(&self, token: &str, channel: &mut Channel) -> Result<Vec<Delivery>, Error> {
        let control = self.control(token).await?;
        let mut deliveries = Vec::new();
        if let Some(drain) = control.drain
            && channel.drain != Some(drain.request_id)
        {
            channel.drain = Some(drain.request_id);
            channel.pulling = false;
            deliveries.push(Delivery::Drain {
                request_id: drain.request_id,
            });
        }
        let wallpaper = wallpaper_revision(&self.pool, &control.actor).await?;
        if channel
            .wallpaper
            .replace(wallpaper)
            .is_some_and(|seen| seen != wallpaper)
        {
            deliveries.push(Delivery::WallpaperChanged);
        }
        deliveries.extend(active(&Work::new(self.pool.clone()), channel).await?);
        if channel.pulling && matches!(control.disposition, RuntimeDisposition::Available) {
            deliveries.extend(self.offer(token, &control.actor, channel).await?);
        }
        Ok(deliveries)
    }

    /// The next turn, or once per idle spell the notice that there is none.
    async fn offer(
        &self,
        token: &str,
        actor: &super::RuntimeActor,
        channel: &mut Channel,
    ) -> Result<Option<Delivery>, Error> {
        let mut tx = self.lock_runtime(actor).await?;
        self.recheck(token, actor, &mut tx).await?;
        let claimed = Work::new(self.pool.clone())
            .claim(&mut tx, actor, self.connections.as_ref())
            .await?;
        let delivery = match claimed {
            Some(turn) => {
                channel.idle = false;
                channel.active.insert(turn.turn);
                Some(Delivery::Turn(turn))
            }
            None if !channel.idle && channel.active.is_empty() => {
                channel.idle = true;
                Some(Delivery::Idle {
                    dream_after_seconds: dream_after(&mut tx, actor).await?,
                })
            }
            None => None,
        };
        tx.commit().await?;
        Ok(delivery)
    }
}

/// Cancellations and new steering for the turns the agent is running.
async fn active(work: &Work, channel: &mut Channel) -> Result<Vec<Delivery>, Error> {
    let mut deliveries = Vec::new();
    for turn in channel.active.clone() {
        if channel.cancelled.contains(&turn) {
            continue;
        }
        if work.cancelled(turn).await? {
            channel.cancelled.insert(turn);
            deliveries.push(Delivery::Cancel { turn });
            continue;
        }
        for (id, message) in work.pending_steering(turn).await? {
            if channel.steered.insert(id) {
                deliveries.push(Delivery::Steer { turn, id, message });
            }
        }
    }
    Ok(deliveries)
}

/// The agent's configured idle delay before it may dream (ADR 0025).
async fn dream_after(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: &super::RuntimeActor,
) -> Result<u64, Error> {
    let seconds = sqlx::query_scalar!(
        "
        SELECT idle_delay_seconds
        FROM agents
        WHERE id = $1
        ",
        actor.agent.0,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(seconds.unsigned_abs().into())
}

async fn wallpaper_revision(
    pool: &sqlx::PgPool,
    actor: &super::RuntimeActor,
) -> Result<i64, Error> {
    Ok(sqlx::query_scalar!(
        "SELECT wallpaper_revision FROM agents WHERE id = $1",
        actor.agent.0,
    )
    .fetch_one(pool)
    .await?)
}

/// Sends deliveries in order; false closes the channel.
async fn deliver(socket: &mut WebSocket, deliveries: Result<Vec<Delivery>, Error>) -> bool {
    let deliveries = match deliveries {
        Ok(deliveries) => deliveries,
        Err(Error::Unauthorized) => vec![Delivery::Revoked],
        Err(error) => {
            tracing::warn!(?error, "runtime channel closed");
            return false;
        }
    };
    for delivery in deliveries {
        let revoked = matches!(delivery, Delivery::Revoked);
        let Ok(text) = serde_json::to_string(&delivery) else {
            return false;
        };
        if socket.send(Message::Text(text.into())).await.is_err() || revoked {
            return false;
        }
    }
    true
}
