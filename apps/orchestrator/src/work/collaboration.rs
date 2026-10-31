//! Agent-to-agent work (ADRs 0016, 0072, 0073). A child is a turn in its recipient's
//! conversation with `parent_turn_id` as its ancestry. A parent with awaited children yields when
//! its run ends and is queued again each time one settles, receiving every settled outcome,
//! failed and cancelled ones with their evidence, and which are still open. A completed
//! `send_message` that expects a reply is relayed back to its sender once, unless it answered
//! explicitly. Delegation and cancellation both lock the root of the ancestry first, so a child
//! committed before a cancel is inside its scope and none after it.
use super::{SendMessage, Work, access};
use crate::{
    agents::{AgentId, kubernetes::Error, runtime::RuntimeActor},
    identity::{Actor, IdentityError},
};
use cubebot_protocol::runtime::{Collaborate, Delegation};
use serde::Serialize;
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Ancestors a child may have, counting its parent.
const DEPTH: i32 = 4;
/// Children one turn may send, so a resumed parent cannot fan out without bound.
const BREADTH: i64 = 16;

#[derive(Clone, Serialize, utoipa::ToSchema)]
pub struct Steering {
    pub id: Uuid,
    pub message: String,
    /// The instruction entered the running context; not proof the agent followed it.
    pub incorporated: bool,
    /// Copied from steering of the work that delegated this.
    pub propagated: bool,
}

impl Work {
    /// Creates the child for one tool call, or returns the one that call already created. The
    /// inner error is the refusal the model reads.
    #[tracing::instrument(skip_all, fields(agent = %actor.agent.0, attempt = %actor.attempt.0, turn = %turn, call = %input.call))]
    pub(crate) async fn collaborate(
        &self,
        actor: &RuntimeActor,
        turn: Uuid,
        input: &Collaborate,
    ) -> Result<Result<Uuid, String>, Error> {
        if !valid(input) {
            return Err(Error::InvalidConfiguration);
        }
        let mut tx = self.pool.begin().await?;
        let (root, _, _) = lineage(&mut tx, turn).await?;
        lock_root(&mut tx, root).await?;
        let human = sqlx::query_scalar!(
            "
            SELECT actor_id
            FROM chat_turns
            WHERE id=$1 AND conversation_id=$2 AND status='running' AND attempt_id=$3
            ",
            turn,
            actor.agent.0,
            actor.attempt.0,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let human = human.ok_or(Error::Conflict)?;
        let previous = sqlx::query!(
            "
            SELECT id, parent_turn_id
            FROM chat_turns
            WHERE actor_id=$1 AND request_id=$2
            ",
            human,
            input.call,
        )
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(previous) = previous {
            return Ok((previous.parent_turn_id == Some(turn))
                .then_some(previous.id)
                .ok_or_else(|| "this call id was already used".into()));
        }
        let created = create(&mut tx, actor, turn, human, input).await?;
        tx.commit().await?;
        Ok(created)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, conversation = %conversation, turn = %turn))]
    pub async fn steer(
        &self,
        actor: &Actor,
        conversation: Uuid,
        turn: Uuid,
        input: &SendMessage,
    ) -> Result<Steering, IdentityError> {
        super::validate(input)?;
        let mut tx = self.pool.begin().await?;
        access::lock_actor(&mut tx, actor).await?;
        let agent = access::conversation(&mut tx, actor, conversation, "conversation.send").await?;
        let previous = sqlx::query!(
            "
            SELECT id, turn_id, message
            FROM turn_steering
            WHERE actor_id=$1 AND request_id=$2
            ",
            actor.id,
            input.request_id,
        )
        .fetch_optional(&mut *tx)
        .await?;
        let id = match previous {
            Some(previous) if previous.turn_id == turn && previous.message == input.message => {
                previous.id
            }
            Some(_) => return Err(IdentityError::Conflict),
            None => queue_steering(&mut tx, actor, agent, turn, input).await?,
        };
        let steering = sqlx::query_as!(
            Steering,
            r#"
            SELECT id, message, incorporated_at IS NOT NULL AS "incorporated!", source_steering_id IS NOT NULL AS "propagated!"
            FROM turn_steering
            WHERE id=$1
            "#,
            id,
        )
        .fetch_one(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(steering)
    }

    /// Steering for the turn not yet reported as incorporated, oldest first.
    #[tracing::instrument(skip_all, fields(turn = %turn))]
    pub(crate) async fn pending_steering(&self, turn: Uuid) -> Result<Vec<(Uuid, String)>, Error> {
        Ok(sqlx::query!(
            "
            SELECT id, message
            FROM turn_steering
            WHERE turn_id=$1 AND incorporated_at IS NULL
            ORDER BY created_at, id
            ",
            turn,
        )
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| (row.id, row.message))
        .collect())
    }
}

/// Locks the root of an ancestry, which delegation and cancellation both take first.
async fn lock_root(tx: &mut Transaction<'_, Postgres>, root: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "
        SELECT 1 AS locked
        FROM chat_turns
        WHERE id=$1
        FOR UPDATE
        ",
        root,
    )
    .fetch_all(&mut **tx)
    .await?;
    Ok(())
}

fn valid(input: &Collaborate) -> bool {
    !input.call.is_nil()
        && !input.agent.is_empty()
        && input.agent.len() <= 255
        && !input.message.trim().is_empty()
        && input.message.len() <= 16384
        && !input.message.contains('\0')
}

/// Finished, failed or cancelled work is never reopened by steering.
async fn queue_steering(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    agent: AgentId,
    turn: Uuid,
    input: &SendMessage,
) -> Result<Uuid, IdentityError> {
    let open = sqlx::query_scalar!(
        "
        SELECT id
        FROM chat_turns
        WHERE id=$1 AND conversation_id=$2 AND status IN ('queued','running','delegated')
        FOR UPDATE
        ",
        turn,
        agent.0,
    )
    .fetch_optional(&mut **tx)
    .await?;
    open.ok_or(IdentityError::Conflict)?;
    let id = sqlx::query_scalar!(
        "
        INSERT INTO turn_steering (turn_id, actor_id, request_id, message)
        VALUES ($1, $2, $3, $4)
        RETURNING id
        ",
        turn,
        actor.id,
        input.request_id,
        &input.message,
    )
    .fetch_one(&mut **tx)
    .await?;
    propagate(tx, id).await?;
    Ok(id)
}

/// Copies steering to the open work delegated under its turn, so a correction reaches the
/// whole task. Messages are not delegated work and keep their own direction.
async fn propagate(tx: &mut Transaction<'_, Postgres>, steering: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query!(
        "
        WITH RECURSIVE down(id) AS (
            SELECT c.id FROM chat_turns c JOIN turn_steering s ON s.turn_id=c.parent_turn_id WHERE s.id=$1 AND c.awaited
            UNION ALL
            SELECT t.id FROM chat_turns t JOIN down d ON t.parent_turn_id=d.id WHERE t.awaited
        )
        INSERT INTO turn_steering (turn_id, actor_id, request_id, message, source_steering_id)
        SELECT c.id, s.actor_id, gen_random_uuid(), s.message, s.id
        FROM chat_turns c
        JOIN turn_steering s ON s.id=$1
        WHERE c.id IN (SELECT id FROM down) AND c.status IN ('queued','running','delegated')
        ON CONFLICT DO NOTHING
        ",
        steering,
    )
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// The coordinator's continuation: the room message becomes steering for an open delivery of
/// the same room. Returns the delivery's agent, or `None` when it is not one.
pub(super) async fn continue_task(
    tx: &mut Transaction<'_, Postgres>,
    message: Uuid,
    task: Uuid,
) -> Result<Option<String>, sqlx::Error> {
    let target = sqlx::query!(
        "
        SELECT t.id, a.name
        FROM chat_turns t
        JOIN agents a ON a.id=t.conversation_id
        JOIN room_messages d ON d.id=t.room_message_id
        JOIN room_messages m ON m.id=$1 AND m.room_id=d.room_id
        WHERE t.id=$2 AND t.status IN ('queued','running','delegated')
        FOR UPDATE OF t
        ",
        message,
        task,
    )
    .fetch_optional(&mut **tx)
    .await?;
    let Some(target) = target else {
        return Ok(None);
    };
    // The room message's id is the steering's request id, so a repeated decision adds nothing.
    let id = sqlx::query_scalar!(
        "
        INSERT INTO turn_steering (turn_id, actor_id, request_id, message)
        SELECT $2, actor_id, id, message
        FROM room_messages
        WHERE id=$1
        ON CONFLICT DO NOTHING
        RETURNING id
        ",
        message,
        target.id,
    )
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(id) = id {
        propagate(tx, id).await?;
    }
    Ok(Some(target.name))
}

/// Relays a completed message's response to its sender as a new turn there, at most once,
/// when the sender asked for a reply and the recipient did not already answer it with
/// `send_message`. The relay expects no reply, so nothing echoes back.
pub(super) async fn relay(
    tx: &mut Transaction<'_, Postgres>,
    turn: Uuid,
) -> Result<(), sqlx::Error> {
    let sender = sqlx::query_scalar!(
        r#"
        INSERT INTO chat_turns (conversation_id, actor_id, request_id, message, parent_turn_id)
        SELECT p.conversation_id, c.actor_id, gen_random_uuid(), format('Reply from agent %s to your message:%s%s',a.name,E'\n\n',left(c.response,16000)), c.id
        FROM chat_turns c
        JOIN chat_turns p ON p.id=c.parent_turn_id
        JOIN agents a ON a.id=c.conversation_id
        WHERE c.id=$1 AND c.status='complete' AND c.expects_reply AND NOT c.awaited
            AND NOT EXISTS(SELECT 1 FROM chat_turns r WHERE r.parent_turn_id=c.id AND r.conversation_id=p.conversation_id)
        RETURNING conversation_id AS "conversation_id: AgentId"
        "#,
        turn,
    )
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(agent) = sender {
        let _ = super::wake(tx, agent).await;
    }
    Ok(())
}

async fn create(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    turn: Uuid,
    human: Uuid,
    input: &Collaborate,
) -> Result<Result<Uuid, String>, Error> {
    let (_, depth, cancelled) = lineage(tx, turn).await?;
    if cancelled {
        return Ok(Err("this work was cancelled; nothing was sent".into()));
    }
    if depth >= DEPTH {
        return Ok(Err(
            "work is already delegated as deeply as allowed; do it yourself".into(),
        ));
    }
    let targets = sqlx::query_scalar!(
        "
        SELECT a.id
        FROM agents a
        WHERE a.name=$2 AND a.id<>$1 AND a.desired_state<>'retired'
            AND EXISTS(SELECT 1 FROM room_members m JOIN room_members n ON n.room_id=m.room_id WHERE m.agent_id=$1 AND n.agent_id=a.id)
        ",
        actor.agent.0,
        &input.agent,
    )
    .fetch_all(&mut **tx)
    .await?;
    let [target] = targets[..] else {
        return Ok(Err(if targets.is_empty() {
            "no other agent with that name shares a room with you"
        } else {
            "several agents share that name; nothing was sent"
        }
        .into()));
    };
    if let Some(reason) = repeated(tx, turn, target, input).await? {
        return Ok(Err(reason));
    }
    super::wake(tx, AgentId(target))
        .await
        .map_err(|_| Error::Conflict)?;
    Ok(Ok(
        insert_child(tx, actor, turn, human, target, input).await?
    ))
}

/// The child turn in the recipient's conversation, worded with the sender's name.
async fn insert_child(
    tx: &mut Transaction<'_, Postgres>,
    actor: &RuntimeActor,
    turn: Uuid,
    human: Uuid,
    target: Uuid,
    input: &Collaborate,
) -> Result<Uuid, sqlx::Error> {
    let verb = if input.wait {
        "Delegated to you"
    } else {
        "Message"
    };
    sqlx::query_scalar!(
        r#"
        INSERT INTO chat_turns (conversation_id, actor_id, request_id, message, parent_turn_id, awaited, expects_reply)
        SELECT $1, $2, $3, format('%s by agent %s:%s%s',$4::text,a.name,E'\n\n',$5::text), $6, $7, $9
        FROM agents a
        WHERE a.id=$8
        RETURNING id
        "#,
        target,
        human,
        input.call,
        verb,
        &input.message,
        turn,
        input.wait,
        actor.agent.0,
        !input.wait && input.expects_reply,
    )
    .fetch_one(&mut **tx)
    .await
}

/// A resumed parent rereads its original request, which can ask it to delegate again: the
/// same task to the same agent is refused with the task it already is, and one request
/// sends at most `BREADTH` children.
async fn repeated(
    tx: &mut Transaction<'_, Postgres>,
    turn: Uuid,
    target: Uuid,
    input: &Collaborate,
) -> Result<Option<String>, sqlx::Error> {
    let sent = sqlx::query!(
        r#"
        SELECT
            (SELECT id FROM chat_turns WHERE parent_turn_id=$1 AND conversation_id=$2 AND awaited=$3 AND right(message,length($4))=$4 ORDER BY sequence LIMIT 1) AS "same?",
            (SELECT count(*) FROM chat_turns WHERE parent_turn_id=$1) AS "sent!"
        "#,
        turn,
        target,
        input.wait,
        &input.message,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(match sent.same {
        Some(task) => Some(format!(
            "this exact work was already sent as task {task}; its outcome comes back to you, so do not send it again"
        )),
        None if sent.sent >= BREADTH => Some(format!(
            "this request has already sent {BREADTH} tasks; finish with what you have"
        )),
        None => None,
    })
}

/// The root of the turn's ancestry, its number of ancestors, and whether any of the turn and
/// its ancestors is cancelled.
async fn lineage(
    tx: &mut Transaction<'_, Postgres>,
    turn: Uuid,
) -> Result<(Uuid, i32, bool), sqlx::Error> {
    let lineage = sqlx::query!(
        r#"
        WITH RECURSIVE up(id,parent,status,depth) AS (
            SELECT id, parent_turn_id, status, 0 FROM chat_turns WHERE id=$1
            UNION ALL
            SELECT t.id, t.parent_turn_id, t.status, u.depth+1 FROM chat_turns t JOIN up u ON t.id=u.parent
        )
        SELECT (SELECT id FROM up WHERE parent IS NULL) AS "root!", max(depth) AS "depth!", bool_or(status='cancelled') AS "cancelled!"
        FROM up
        "#,
        turn,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok((lineage.root, lineage.depth, lineage.cancelled))
}

/// States the agent's own name, so a mention or a message addressed to it reads as its own,
/// and names the agents sharing a room with it, which its collaboration tools can reach.
pub(super) async fn with_peers(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    instructions: String,
) -> Result<String, sqlx::Error> {
    let name = sqlx::query_scalar!(
        "
        SELECT name
        FROM agents
        WHERE id=$1
        ",
        agent.0,
    )
    .fetch_one(&mut **tx)
    .await?;
    let instructions = format!("{instructions}\n\nYou are the agent named {name}.");
    let peers = sqlx::query!(
        r#"
        SELECT a.name, string_agg(DISTINCT r.name,', ' ORDER BY r.name) AS "rooms!"
        FROM room_members m
        JOIN room_members n ON n.room_id=m.room_id AND n.agent_id<>m.agent_id
        JOIN agents a ON a.id=n.agent_id AND a.desired_state<>'retired'
        JOIN rooms r ON r.id=m.room_id
        WHERE m.agent_id=$1
        GROUP BY a.name
        ORDER BY a.name
        "#,
        agent.0,
    )
    .fetch_all(&mut **tx)
    .await?;
    if peers.is_empty() {
        return Ok(instructions);
    }
    let lines: Vec<String> = peers
        .iter()
        .map(|peer| format!("- {} (rooms: {})", peer.name, peer.rooms))
        .collect();
    Ok(format!(
        "{instructions}\n\nAgents you can reach with delegate and send_message:\n{}",
        lines.join("\n")
    ))
}

/// Where a run that ended with a response leaves its turn: queued for settled outcomes its run
/// did not see, waiting on open children, or complete.
pub(super) async fn after_run(
    tx: &mut Transaction<'_, Postgres>,
    turn: Uuid,
) -> Result<&'static str, sqlx::Error> {
    let children = sqlx::query!(
        r#"
        SELECT
            count(*) FILTER (WHERE status NOT IN ('complete','failed','cancelled')) AS "open!",
            count(*) FILTER (WHERE status IN ('complete','failed','cancelled') AND NOT seen) AS "unseen!"
        FROM chat_turns
        WHERE parent_turn_id=$1 AND awaited
        "#,
        turn,
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(next_state(children.open, children.unseen))
}

fn next_state(open: i64, unseen: i64) -> &'static str {
    if unseen > 0 {
        "queued"
    } else if open > 0 {
        "delegated"
    } else {
        "complete"
    }
}

/// Queues a yielded parent as soon as one of its awaited children settles.
pub(super) async fn resume_parent(
    tx: &mut Transaction<'_, Postgres>,
    child: Uuid,
) -> Result<(), sqlx::Error> {
    let parent = sqlx::query_scalar!(
        r#"
        UPDATE chat_turns p
        SET status='queued'
        FROM chat_turns c
        WHERE c.id=$1 AND c.awaited AND c.status IN ('complete','failed','cancelled') AND p.id=c.parent_turn_id AND p.status='delegated'
        RETURNING p.conversation_id AS "conversation_id: AgentId"
        "#,
        child,
    )
    .fetch_optional(&mut **tx)
    .await?;
    if let Some(agent) = parent {
        // A retired parent agent keeps its queued turn as evidence.
        let _ = super::wake(tx, agent).await;
    }
    Ok(())
}

/// Every awaited child, settled ones with their outcome and evidence, marking those seen by
/// this run; open ones are listed so the parent knows what is still coming.
pub(super) async fn delegations(
    tx: &mut Transaction<'_, Postgres>,
    turn: Uuid,
) -> Result<Vec<Delegation>, Error> {
    sqlx::query!(
        "
        UPDATE chat_turns
        SET seen=true
        WHERE parent_turn_id=$1 AND awaited AND status IN ('complete','failed','cancelled')
        ",
        turn,
    )
    .execute(&mut **tx)
    .await?;
    let rows = sqlx::query!(
        "
        SELECT c.id, a.name, c.status, c.response, c.error
        FROM chat_turns c
        JOIN agents a ON a.id=c.conversation_id
        WHERE c.parent_turn_id=$1 AND c.awaited
        ORDER BY c.sequence
        ",
        turn,
    )
    .fetch_all(&mut **tx)
    .await?;
    let mut outcomes = Vec::with_capacity(rows.len());
    for child in rows {
        let evidence = sqlx::query_scalar!(
            r#"
            SELECT payload #- '{outcome,image}' AS "payload!"
            FROM (SELECT sequence,payload FROM turn_events WHERE turn_id=$1 AND payload->>'kind' IN ('tool_call','tool_result') ORDER BY sequence DESC LIMIT 8) p
            ORDER BY sequence
            "#,
            child.id,
        )
        .fetch_all(&mut **tx)
        .await?;
        outcomes.push(Delegation {
            turn: child.id,
            agent: child.name,
            status: child.status,
            response: child.response,
            error: child.error,
            evidence: evidence
                .into_iter()
                .map(serde_json::from_value)
                .collect::<Result<_, _>>()
                .map_err(|error| Error::Configuration(error.into()))?,
        });
    }
    Ok(outcomes)
}

/// Cancels the turn and every unsettled descendant; the parent receives the cancelled outcome.
pub(super) async fn cancel_subtree(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    turn: Uuid,
) -> Result<(), IdentityError> {
    let (root, _, _) = lineage(tx, turn).await?;
    lock_root(tx, root).await?;
    // Cancelling again is accepted; finished results stay what they were.
    let changed = sqlx::query!(
        "
        UPDATE chat_turns
        SET status='cancelled', error='cancelled'
        WHERE id=$1 AND conversation_id=$2 AND status NOT IN ('complete','failed')
        ",
        turn,
        agent.0,
    )
    .execute(&mut **tx)
    .await?;
    if changed.rows_affected() != 1 {
        return Err(IdentityError::Conflict);
    }
    sqlx::query!(
        "
        WITH RECURSIVE down(id) AS (
            SELECT id FROM chat_turns WHERE parent_turn_id=$1
            UNION ALL
            SELECT t.id FROM chat_turns t JOIN down d ON t.parent_turn_id=d.id
        )
        UPDATE chat_turns
        SET status='cancelled', error='cancelled'
        WHERE id IN (SELECT id FROM down) AND status NOT IN ('complete','failed','cancelled')
        ",
        turn,
    )
    .execute(&mut **tx)
    .await?;
    resume_parent(tx, turn).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_parent_runs_again_for_any_unseen_outcome_and_waits_only_on_open_children() {
        assert_eq!(super::next_state(0, 0), "complete");
        assert_eq!(super::next_state(2, 0), "delegated");
        assert_eq!(super::next_state(1, 1), "queued");
        assert_eq!(super::next_state(0, 3), "queued");
    }
}
