use crate::{connections::Connections, identity::IdentityError, work::Work};
use sqlx::PgPool;
use uuid::Uuid;

const LINES: i64 = 1500;
const UNAVAILABLE: &str = "A summary was not available: no model provider could be reached.";
const SUMMARIZER: &str = "You write the summary of a meeting from its transcript. You have no tools. Write plain markdown under 250 words with short sections for decisions, action items with their owners, and open questions; leave out a section with nothing in it. Use only what the transcript says, and never use em dashes.";

#[tracing::instrument(skip_all, fields(meeting = %meeting))]
pub(crate) async fn summarize(pool: PgPool, connections: Option<Connections>, meeting: Uuid) {
    if let Err(error) = write(&pool, connections.as_ref(), meeting).await {
        tracing::warn!(?error, "meeting summary not stored");
    }
}

async fn write(
    pool: &PgPool,
    connections: Option<&Connections>,
    meeting: Uuid,
) -> Result<(), IdentityError> {
    let held = sqlx::query!(
        r#"
        SELECT m.title, m.owner_id, t.lines AS "lines!"
        FROM meetings m,
        LATERAL (
            SELECT COALESCE(string_agg(line, E'\n' ORDER BY sequence), '') AS lines
            FROM (
                SELECT u.sequence, COALESCE(NULLIF(h.profile->>'name', ''), h.login, g.name,
                    u.guest_name || ' (guest)') || ': ' || u.text AS line
                FROM meeting_utterances u
                LEFT JOIN human_accounts h ON h.id=u.account_id
                LEFT JOIN agents g ON g.id=u.agent_id
                WHERE u.meeting_id=m.id
                ORDER BY u.sequence DESC
                LIMIT $2
            ) recent
        ) t
        WHERE m.id=$1
        "#,
        meeting,
        LINES,
    )
    .fetch_optional(pool)
    .await?;
    let Some(held) = held.filter(|held| !held.lines.is_empty()) else {
        return Ok(());
    };
    let prompt = format!(
        "Meeting: {}\n\nTranscript, oldest first:\n{}",
        held.title, held.lines
    );
    let answer =
        crate::work::ask_as(pool, connections, held.owner_id, (SUMMARIZER, &prompt), 600).await;
    let summary = match answer.as_deref().map(str::trim) {
        Ok(summary) if !summary.is_empty() => summary.to_owned(),
        _ => UNAVAILABLE.to_owned(),
    };
    sqlx::query!(
        "UPDATE meetings SET summary=$2 WHERE id=$1",
        meeting,
        summary
    )
    .execute(pool)
    .await?;
    Work::new(pool.clone())
        .meeting_ended(meeting, &summary)
        .await
}
