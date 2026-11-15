use crate::{
    calendar::{Item, MeetingDetail, Source, utc},
    identity::IdentityError,
};
use sqlx::PgPool;
use uuid::Uuid;

pub(crate) async fn items(
    pool: &PgPool,
    viewer: Uuid,
    (from, to): (i64, i64),
) -> Result<(Option<Source>, Vec<Item>), IdentityError> {
    let rows = sqlx::query!(
        r#"
        SELECT m.id, m.title, COALESCE(NULLIF(o.profile->>'name', ''), o.login) AS "owner!",
            CASE WHEN m.ended_at IS NOT NULL THEN 'ended'
                WHEN m.started_at IS NOT NULL THEN 'live' ELSE 'scheduled' END AS "state!",
            extract(epoch FROM COALESCE(m.starts_at, m.started_at))::bigint AS "start!",
            extract(epoch FROM COALESCE(m.ends_at, m.ended_at, now()))::bigint AS "end!"
        FROM meetings m
        JOIN human_accounts o ON o.id=m.owner_id
        WHERE (m.owner_id=$1 OR EXISTS(
                SELECT 1 FROM meeting_invitations i WHERE i.meeting_id=m.id AND i.account_id=$1
            ))
            AND COALESCE(m.starts_at, m.started_at) < to_timestamp($3::bigint)
            AND COALESCE(m.ends_at, m.ended_at, now()) > to_timestamp($2::bigint)
        ORDER BY 5, m.id
        "#,
        viewer,
        from,
        to,
    )
    .fetch_all(pool)
    .await?;
    let source = (!rows.is_empty()).then(|| Source {
        id: "meetings".to_owned(),
        kind: "meetings",
        name: "Meetings".to_owned(),
        account: None,
        connector: None,
        connection: None,
        color: None,
        writable: false,
        problem: None,
    });
    let items = rows.into_iter().map(|row| Item {
        kind: "meeting",
        id: format!("meeting:{}", row.id),
        source: "meetings".to_owned(),
        title: row.title,
        start: utc(row.start),
        end: Some(utc(row.end)),
        all_day: false,
        editable: false,
        event: None,
        routine: None,
        meeting: Some(MeetingDetail {
            id: row.id,
            state: row.state,
            owner: row.owner,
            join: format!("/meetings/{}", row.id),
        }),
    });
    Ok((source, items.collect()))
}
