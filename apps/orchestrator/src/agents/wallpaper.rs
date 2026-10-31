//! Per-agent desktop wallpaper: a JPEG or PNG a manager sets, which the agent draws on its desktop.
use super::{AgentId, Agents};
use crate::identity::{Actor, IdentityError};

pub const WALLPAPER_BYTES: usize = 4 * 1024 * 1024;

impl Agents {
    /// Sets the wallpaper, or resets it to the default with `None`; needs `agent.manage`.
    #[tracing::instrument(skip_all, fields(agent = %id.0))]
    pub async fn set_wallpaper(
        &self,
        actor: &Actor,
        id: AgentId,
        image: Option<&[u8]>,
    ) -> Result<(), IdentityError> {
        if image.is_some_and(|image| content_type(image).is_none() || image.len() > WALLPAPER_BYTES)
        {
            return Err(IdentityError::InvalidInput);
        }
        let updated = sqlx::query!(
            "
            UPDATE agents g
            SET wallpaper = $3, wallpaper_revision = g.wallpaper_revision + 1
            FROM human_accounts a
            WHERE a.id = $1
              AND a.active
              AND g.id = $2
              AND (
                a.role = 'administrator'
                OR EXISTS(
                    SELECT 1
                    FROM effective_human_grants p
                    WHERE p.actor_id = a.id
                      AND p.scope_kind = 'agent'
                      AND p.scope_id = g.id
                      AND p.capability = 'agent.manage'
                )
              )
            ",
            actor.id,
            id.0,
            image,
        )
        .execute(&self.pool)
        .await?;
        if updated.rows_affected() == 0 {
            return Err(IdentityError::NotFound);
        }
        Ok(())
    }
}

/// The agent's own wallpaper, if one is set.
pub async fn wallpaper(
    pool: &sqlx::PgPool,
    agent: AgentId,
) -> Result<Option<Vec<u8>>, sqlx::Error> {
    Ok(
        sqlx::query_scalar!("SELECT wallpaper FROM agents WHERE id = $1", agent.0)
            .fetch_optional(pool)
            .await?
            .flatten(),
    )
}

/// The image type by its magic bytes; anything else is refused.
pub fn content_type(image: &[u8]) -> Option<&'static str> {
    if image.starts_with(&[0xff, 0xd8, 0xff]) {
        Some("image/jpeg")
    } else if image.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_jpeg_and_png_magic_bytes_are_images() {
        use super::content_type;
        assert_eq!(content_type(b"\xff\xd8\xff\xe0rest"), Some("image/jpeg"));
        assert_eq!(content_type(b"\x89PNG\r\n\x1a\nrest"), Some("image/png"));
        assert_eq!(content_type(b"GIF89a"), None);
        assert_eq!(content_type(b"<svg"), None);
        assert_eq!(content_type(b""), None);
    }
}
