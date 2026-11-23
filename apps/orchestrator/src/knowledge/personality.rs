use super::{Knowledge, authorize};
use crate::{
    agents::AgentId,
    identity::{Actor, IdentityError},
};
use cubebot_protocol::runtime::Outreach;
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, Transaction};

pub(crate) const PERSONALITY_CHARS: usize = 800;

#[derive(Serialize, utoipa::ToSchema)]
pub struct Personality {
    pub personality: String,
    pub updated_at: Option<String>,
    pub edited_by: Option<String>,
    pub limit: usize,
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SavePersonality {
    pub personality: String,
}

impl Knowledge {
    #[tracing::instrument(skip_all, fields(actor = %actor.id, agent = %agent.0))]
    pub async fn personality(
        &self,
        actor: &Actor,
        agent: AgentId,
    ) -> Result<Personality, IdentityError> {
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, actor, agent, "memory.read").await?;
        let personality = read(&mut tx, agent).await?;
        tx.commit().await?;
        Ok(personality)
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id, agent = %agent.0))]
    pub async fn save_personality(
        &self,
        actor: &Actor,
        agent: AgentId,
        input: &SavePersonality,
    ) -> Result<Personality, IdentityError> {
        let text = input.personality.trim();
        if !fits(text) {
            return Err(IdentityError::InvalidInput);
        }
        let mut tx = self.pool.begin().await?;
        authorize(&mut tx, actor, agent, "memory.edit").await?;
        sqlx::query!(
            "UPDATE agents SET personality=$2, personality_updated_at=clock_timestamp(), personality_edited_by=$3 WHERE id=$1",
            agent.0,
            text,
            actor.id,
        )
        .execute(&mut *tx)
        .await?;
        let personality = read(&mut tx, agent).await?;
        tx.commit().await?;
        Ok(personality)
    }
}

fn fits(text: &str) -> bool {
    text.chars().count() <= PERSONALITY_CHARS && !text.contains('\0')
}

async fn read(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
) -> Result<Personality, IdentityError> {
    let row = sqlx::query!(
        r#"
        SELECT g.personality, (to_jsonb(g.personality_updated_at) #>> '{}') AS updated_at, h.login AS "edited_by?"
        FROM agents g
        LEFT JOIN human_accounts h ON h.id=g.personality_edited_by
        WHERE g.id=$1
        "#,
        agent.0,
    )
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(IdentityError::NotFound)?;
    Ok(Personality {
        personality: row.personality,
        updated_at: row.updated_at,
        edited_by: row.edited_by,
        limit: PERSONALITY_CHARS,
    })
}

pub(crate) async fn persona(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
) -> Result<String, sqlx::Error> {
    let row = sqlx::query!(
        "SELECT instructions, personality FROM agents WHERE id=$1",
        agent.0
    )
    .fetch_one(&mut **tx)
    .await?;
    Ok(introduced(row.instructions, &row.personality))
}

fn introduced(instructions: String, personality: &str) -> String {
    if personality.trim().is_empty() {
        return instructions;
    }
    format!(
        "{instructions}\n\nWho you are, in your own words. You wrote this and keep growing it, so let it shape how you think, write and act; your instructions and the person's requests still come first:\n{personality}"
    )
}

pub(crate) async fn grow(
    tx: &mut Transaction<'_, Postgres>,
    agent: AgentId,
    personality: Option<&str>,
    outreach: Option<&Outreach>,
) -> Result<bool, IdentityError> {
    let mut grew = false;
    if let Some(text) = personality
        .map(str::trim)
        .filter(|text| !text.is_empty() && fits(text))
    {
        sqlx::query!(
            "UPDATE agents SET personality=$2, personality_updated_at=clock_timestamp(), personality_edited_by=NULL WHERE id=$1",
            agent.0,
            text,
        )
        .execute(&mut **tx)
        .await?;
        grew = true;
    }
    if let Some(outreach) = outreach.filter(|outreach| reachable(&outreach.intent)) {
        let minutes = i64::from(outreach.after_minutes);
        grew |= crate::work::wakes::reach_out(tx, agent, minutes, outreach.intent.trim()).await?;
    }
    Ok(grew)
}

fn reachable(intent: &str) -> bool {
    let intent = intent.trim();
    !intent.is_empty() && intent.len() <= 2000 && !intent.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_personality_leaves_the_instructions_alone() {
        assert_eq!(introduced("Be useful".into(), "  "), "Be useful");
        let grown = introduced("Be useful".into(), "Dry humor, short sentences.");
        assert!(grown.starts_with("Be useful\n\nWho you are, in your own words."));
        assert!(grown.ends_with("Dry humor, short sentences."));
    }

    #[test]
    fn a_personality_fits_eight_hundred_characters_without_nul() {
        assert!(fits(&"é".repeat(PERSONALITY_CHARS)));
        assert!(!fits(&"a".repeat(PERSONALITY_CHARS + 1)));
        assert!(!fits("a\0b"));
        assert!(!reachable(" "));
        assert!(reachable("Share the release notes"));
    }
}
