//! Profile pictures (migrations/0045_account_avatars.sql): each person sets their own; every
//! signed-in person may see them, as they see each other's names in rooms.
use super::{
    Actor, Identity, IdentityError,
    audit::{Event, record},
};
use serde::Serialize;
use uuid::Uuid;

/// The largest picture accepted; the console sends a 256 px square well under it.
pub const AVATAR_BYTES: usize = 1024 * 1024;

#[derive(Serialize, utoipa::ToSchema)]
pub struct AvatarRevision {
    pub account: Uuid,
    pub login: String,
    /// Changes with each new picture; part of the picture's URL so it can be cached.
    pub revision: i64,
}

impl Identity {
    /// Replaces the actor's picture, or removes it for `None`; returns the new revision.
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn set_avatar(
        &self,
        actor: &Actor,
        image: Option<&[u8]>,
    ) -> Result<i64, IdentityError> {
        if let Some(image) = image
            && (image.len() > AVATAR_BYTES || crate::agents::content_type(image).is_none())
        {
            return Err(IdentityError::InvalidInput);
        }
        let mut transaction = self.pool.begin().await?;
        let revision = sqlx::query_scalar!(
            "
            UPDATE human_accounts
            SET avatar = $2, avatar_revision = avatar_revision + 1
            WHERE id = $1 AND active
            RETURNING avatar_revision
            ",
            actor.id,
            image,
        )
        .fetch_optional(&mut *transaction)
        .await?
        .ok_or(IdentityError::Unauthorized)?;
        let detail = serde_json::json!({ "removed": image.is_none() });
        let event = Event::by(actor.id, "account.avatar").on(actor.id);
        record(&mut *transaction, &event.detail(detail)).await?;
        transaction.commit().await?;
        Ok(revision)
    }

    /// Active accounts with a picture; the caller is signed in.
    pub async fn avatars(&self) -> Result<Vec<AvatarRevision>, IdentityError> {
        Ok(sqlx::query_as!(
            AvatarRevision,
            "
            SELECT id AS account, login, avatar_revision AS revision
            FROM human_accounts
            WHERE active AND avatar IS NOT NULL
            ",
        )
        .fetch_all(&self.pool)
        .await?)
    }

    /// One account's picture; the caller is signed in.
    pub async fn avatar(&self, account: Uuid) -> Result<Option<Vec<u8>>, IdentityError> {
        Ok(sqlx::query_scalar!(
            "SELECT avatar FROM human_accounts WHERE id = $1 AND active",
            account,
        )
        .fetch_optional(&self.pool)
        .await?
        .flatten())
    }
}
