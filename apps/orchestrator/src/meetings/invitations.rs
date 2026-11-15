use super::{Invite, Meetings};
use crate::{
    connections::{ConnectError, Connections},
    identity::{Actor, IdentityError},
    work::access,
};
use chrono::DateTime;
use chrono_tz::Tz;
use cubebot_connectors::Level;
use futures_util::future::join_all;
use serde::Serialize;
use serde_json::{Value, json};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

const MOST_EMAILS: usize = 50;
const ERROR_CHARS: usize = 200;
const NO_LINK: &str = "The meeting has no public link to send.";

#[derive(Serialize, utoipa::ToSchema)]
pub struct EmailConnection {
    pub id: Uuid,
    pub slug: String,
    pub connector: String,
    pub account: Option<String>,
}

#[derive(Serialize)]
pub struct Invited {
    pub name: String,
    pub accounts: usize,
    pub agents: usize,
    pub emails: usize,
}

pub(super) struct Added {
    accounts: usize,
    agents: usize,
    emails: Vec<(Uuid, String)>,
}

impl Added {
    pub(super) fn counts(&self, name: String) -> Invited {
        Invited {
            name,
            accounts: self.accounts,
            agents: self.agents,
            emails: self.emails.len(),
        }
    }
}

pub(super) struct Sender {
    connection: Uuid,
    tool: &'static str,
    body: &'static str,
}

struct Letter {
    title: String,
    description: String,
    starts: Option<i64>,
    ends: Option<i64>,
    token: Option<String>,
    zone: Option<String>,
}

impl Meetings {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn email_connections(
        &self,
        actor: &Actor,
    ) -> Result<Vec<EmailConnection>, IdentityError> {
        let senders = self.senders(actor.id, None).await?;
        Ok(senders
            .into_iter()
            .map(|(connection, _)| connection)
            .collect())
    }

    pub(super) async fn sender(
        &self,
        actor: &Actor,
        connection: Option<Uuid>,
    ) -> Result<Option<Sender>, IdentityError> {
        let Some(connection) = connection else {
            return Ok(None);
        };
        let senders = self.senders(actor.id, Some(connection)).await?;
        let (_, sender) = senders.into_iter().next().ok_or(IdentityError::NotFound)?;
        Ok(Some(sender))
    }

    async fn senders(
        &self,
        viewer: Uuid,
        one: Option<Uuid>,
    ) -> Result<Vec<(EmailConnection, Sender)>, IdentityError> {
        if self.connections.is_none() {
            return Ok(Vec::new());
        }
        let rows = sqlx::query!(
            r#"
            SELECT id, connector, slug, account, config, level,
                disabled_tools AS "disabled_tools!: sqlx::types::Json<Vec<String>>"
            FROM connections
            WHERE created_by=$1 AND ($2::uuid IS NULL OR id=$2)
                AND connector IN ('google', 'microsoft', 'email')
            ORDER BY connector, slug
            "#,
            viewer,
            one,
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let (tool, body) = sending(&row.connector)?;
                let manifest = cubebot_connectors::find(&row.connector)?.manifest();
                let level = Level::parse(&row.level)?;
                manifest
                    .offered(&row.config, level, &row.disabled_tools.0)
                    .any(|offered| offered.name == tool)
                    .then_some(())?;
                let sender = Sender {
                    connection: row.id,
                    tool,
                    body,
                };
                let connection = EmailConnection {
                    id: row.id,
                    slug: row.slug,
                    connector: row.connector,
                    account: row.account,
                };
                Some((connection, sender))
            })
            .collect())
    }

    pub(super) async fn add(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        actor: &Actor,
        meeting: Uuid,
        invite: &Invite,
        emails: &[String],
        sender: Option<&Sender>,
    ) -> Result<Added, IdentityError> {
        let agents = agents(tx, actor, &invite.agents).await?;
        accounts(tx, &invite.accounts).await?;
        let rows = sqlx::query!(
            r#"
            INSERT INTO meeting_invitations
                (meeting_id, account_id, agent_id, email, invited_by, email_connection_id)
            SELECT $1::uuid, account, NULL::uuid, NULL, $2::uuid, NULL::uuid
            FROM unnest($3::uuid[]) account
            UNION ALL SELECT $1, NULL, agent, NULL, $2, NULL FROM unnest($4::uuid[]) agent
            UNION ALL SELECT $1, NULL, NULL, email, $2, $6::uuid FROM unnest($5::text[]) email
            ON CONFLICT DO NOTHING
            RETURNING id, account_id, agent_id, email
            "#,
            meeting,
            actor.id,
            &invite.accounts,
            &agents,
            emails,
            sender.map(|sender| sender.connection),
        )
        .fetch_all(&mut **tx)
        .await?;
        Ok(Added {
            accounts: rows.iter().filter(|row| row.account_id.is_some()).count(),
            agents: rows.iter().filter(|row| row.agent_id.is_some()).count(),
            emails: rows
                .into_iter()
                .filter_map(|row| Some((row.id, row.email?)))
                .collect(),
        })
    }

    pub(super) async fn deliver(
        &self,
        actor: &Actor,
        meeting: Uuid,
        sender: Option<&Sender>,
        added: &Added,
    ) -> Result<(), IdentityError> {
        let (Some(sender), Some(connections)) = (sender, &self.connections) else {
            return Ok(());
        };
        if added.emails.is_empty() {
            return Ok(());
        }
        let letter = sqlx::query_as!(
            Letter,
            r#"
            SELECT m.title, m.description, extract(epoch FROM m.starts_at)::bigint AS starts,
                extract(epoch FROM m.ends_at)::bigint AS ends, m.link_token AS token,
                h.profile->>'time_zone' AS zone
            FROM meetings m
            JOIN human_accounts h ON h.id=$2
            WHERE m.id=$1
            "#,
            meeting,
            actor.id,
        )
        .fetch_one(&self.pool)
        .await?;
        let link = self.origin.as_deref().zip(letter.token.as_deref());
        let text = link.map(|(origin, token)| body(&letter, &format!("{origin}/m/{token}")));
        let subject = format!("Invitation: {}", letter.title);
        let sends = added.emails.iter().map(|(invitation, to)| {
            let input = text.as_deref().map(|text| {
                let mut input = json!({"to": [to], "subject": subject});
                input[sender.body] = json!(text);
                input
            });
            self.send(connections, sender, *invitation, input)
        });
        join_all(sends).await;
        Ok(())
    }

    async fn send(
        &self,
        connections: &Connections,
        sender: &Sender,
        invitation: Uuid,
        input: Option<Value>,
    ) {
        let outcome = match input {
            Some(input) => connections
                .call_tool(sender.connection, sender.tool, input)
                .await
                .map(drop)
                .map_err(|error| failure(&error)),
            None => Err(NO_LINK.to_owned()),
        };
        let recorded = sqlx::query!(
            "
            UPDATE meeting_invitations
            SET emailed_at=CASE WHEN $2::text IS NULL THEN now() END, email_error=$2
            WHERE id=$1
            ",
            invitation,
            outcome.as_ref().err().map(String::as_str),
        )
        .execute(&self.pool)
        .await;
        if let Err(error) = recorded {
            tracing::warn!(%error, %invitation, "invitation email outcome not recorded");
        }
    }
}

fn sending(connector: &str) -> Option<(&'static str, &'static str)> {
    match connector {
        "google" => Some(("gmail_send", "text")),
        "microsoft" => Some(("mail_send", "body")),
        "email" => Some(("send", "text")),
        _ => None,
    }
}

fn failure(error: &ConnectError) -> String {
    let message = match error {
        ConnectError::Invalid(message)
        | ConnectError::Refused(message)
        | ConnectError::Unavailable(message) => message.as_str(),
        ConnectError::SignInAgain => "The account refused the credential; sign in again.",
        ConnectError::Identity(_) => "The email could not be sent.",
    };
    message.chars().take(ERROR_CHARS).collect()
}

async fn agents(
    tx: &mut Transaction<'_, Postgres>,
    actor: &Actor,
    agents: &[Uuid],
) -> Result<Vec<Uuid>, IdentityError> {
    if agents.is_empty() {
        return Ok(Vec::new());
    }
    let joins = sqlx::query_scalar!("SELECT agents_join FROM meeting_settings")
        .fetch_one(&mut **tx)
        .await?;
    if !joins {
        return Err(IdentityError::Forbidden);
    }
    let mut allowed = Vec::with_capacity(agents.len());
    for agent in agents {
        allowed.push(
            access::conversation(tx, actor, *agent, "conversation.send")
                .await?
                .0,
        );
    }
    let retired = sqlx::query_scalar!(
        r#"SELECT EXISTS(SELECT 1 FROM agents WHERE id=ANY($1) AND desired_state='retired') AS "retired!""#,
        &allowed,
    )
    .fetch_one(&mut **tx)
    .await?;
    if retired {
        return Err(IdentityError::Conflict);
    }
    Ok(allowed)
}

async fn accounts(
    tx: &mut Transaction<'_, Postgres>,
    accounts: &[Uuid],
) -> Result<(), IdentityError> {
    let active = sqlx::query_scalar!(
        r#"
        SELECT NOT EXISTS(
            SELECT 1 FROM unnest($1::uuid[]) wanted
            WHERE NOT EXISTS(SELECT 1 FROM human_accounts a WHERE a.id=wanted AND a.active)
        ) AS "active!"
        "#,
        accounts,
    )
    .fetch_one(&mut **tx)
    .await?;
    active.then_some(()).ok_or(IdentityError::InvalidInput)
}

pub(super) fn addresses(emails: &[String]) -> Result<Vec<String>, IdentityError> {
    if emails.len() > MOST_EMAILS {
        return Err(IdentityError::InvalidInput);
    }
    let mut unique: Vec<String> = Vec::with_capacity(emails.len());
    for email in emails {
        let email = address(email).ok_or(IdentityError::InvalidInput)?;
        if !unique.contains(&email) {
            unique.push(email);
        }
    }
    Ok(unique)
}

fn address(raw: &str) -> Option<String> {
    let email = raw.trim().to_lowercase();
    let (local, domain) = email.split_once('@')?;
    let local_valid = (1..=64).contains(&local.len())
        && !local.starts_with('.')
        && !local.ends_with('.')
        && local
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._%+-'".contains(&byte));
    let label = |label: &str| {
        (1..=63).contains(&label.len())
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    };
    let domain_valid = domain.len() <= 253 && domain.contains('.') && domain.split('.').all(label);
    (local_valid && domain_valid).then_some(email)
}

fn body(letter: &Letter, link: &str) -> String {
    let when = match letter.starts.zip(letter.ends) {
        Some(span) => during(span, letter.zone.as_deref()),
        None => "now".to_owned(),
    };
    let mut text = format!(
        "You are invited to the meeting \"{}\".\n\nWhen: {when}\n",
        letter.title
    );
    if !letter.description.is_empty() {
        text.push_str(&format!("\n{}\n", letter.description));
    }
    text.push_str(&format!(
        "\nJoin: {link}\n\nThis meeting is transcribed, and AI agents may take part.\n"
    ));
    text
}

fn during((starts, ends): (i64, i64), zone: Option<&str>) -> String {
    let zone: Tz = zone.and_then(|zone| zone.parse().ok()).unwrap_or(Tz::UTC);
    let at = |seconds| DateTime::from_timestamp(seconds, 0).map(|at| at.with_timezone(&zone));
    let (Some(starts), Some(ends)) = (at(starts), at(ends)) else {
        return "now".to_owned();
    };
    let day = "%a %-d %b %Y, %H:%M";
    let end = match starts.date_naive() == ends.date_naive() {
        true => ends.format("%H:%M"),
        false => ends.format(day),
    };
    format!("{} to {end} ({})", starts.format(day), zone.name())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_trimmed_lowercased_and_deduplicated() {
        let given = [
            " Ana@Example.com ",
            "ana@example.com",
            "bo+x@mail.example.org",
        ]
        .map(String::from);
        assert_eq!(
            addresses(&given).unwrap(),
            ["ana@example.com", "bo+x@mail.example.org"]
        );
    }

    #[test]
    fn malformed_addresses_and_long_lists_are_refused() {
        for bad in [
            "ana",
            "@example.com",
            "ana@",
            "ana@example",
            "ana@@example.com",
            "Ana <ana@example.com>",
            "ana@example.com, bo@example.com",
            "ana@exa mple.com",
            ".ana@example.com",
            "ana@-example.com",
            "ana@example.com\r\nBcc: x@example.com",
        ] {
            assert!(address(bad).is_none(), "{bad}");
        }
        let many: Vec<String> = (0..51).map(|n| format!("p{n}@example.com")).collect();
        assert!(addresses(&many).is_err());
        assert_eq!(addresses(&many[..50]).unwrap().len(), 50);
    }

    #[test]
    fn the_letter_names_the_time_in_the_inviters_zone_and_the_link() {
        let letter = Letter {
            title: "Planning".into(),
            description: "Bring numbers.".into(),
            starts: Some(1_794_823_200),
            ends: Some(1_794_826_800),
            token: None,
            zone: Some("Europe/Lisbon".into()),
        };
        let text = body(&letter, "https://meet.example.com/m/abc");
        assert!(text.contains("When: Mon 16 Nov 2026, 10:00 to 11:00 (Europe/Lisbon)"));
        assert!(text.contains("Bring numbers."));
        assert!(text.contains("Join: https://meet.example.com/m/abc"));
        assert!(text.contains("transcribed"));
        assert!(!text.contains('\u{2014}'));
        let instant = Letter {
            starts: None,
            ends: None,
            zone: None,
            ..letter
        };
        assert!(body(&instant, "x").contains("When: now"));
    }
}
