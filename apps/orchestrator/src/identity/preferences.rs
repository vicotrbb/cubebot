use super::{Actor, Identity, IdentityError};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    #[serde(default)]
    pub favorites: Vec<Favorite>,
    #[serde(default)]
    pub theme: Theme,
    /// Sidebar sections the person collapsed.
    #[serde(default)]
    pub collapsed: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Favorite {
    pub kind: FavoriteKind,
    pub id: Uuid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FavoriteKind {
    Agent,
    Room,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

impl Identity {
    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn preferences(&self, actor: &Actor) -> Result<Preferences, IdentityError> {
        Ok(sqlx::query_scalar!(
            r#"
            SELECT document AS "document: sqlx::types::Json<Preferences>"
            FROM account_preferences
            WHERE account_id = $1
            "#,
            actor.id,
        )
        .fetch_optional(&self.pool)
        .await?
        .map(|document| document.0)
        .unwrap_or_default())
    }

    #[tracing::instrument(skip_all, fields(actor = %actor.id))]
    pub async fn set_preferences(
        &self,
        actor: &Actor,
        preferences: &Preferences,
    ) -> Result<(), IdentityError> {
        if preferences.favorites.len() > 200
            || preferences.collapsed.len() > 100
            || preferences
                .collapsed
                .iter()
                .any(|section| section.len() > 128 || section.chars().any(char::is_control))
        {
            return Err(IdentityError::InvalidInput);
        }
        let stored = sqlx::query!(
            "
            INSERT INTO account_preferences (account_id, document)
            SELECT id, $2
            FROM human_accounts
            WHERE id = $1 AND active
            ON CONFLICT (account_id)
            DO UPDATE SET document = EXCLUDED.document, updated_at = clock_timestamp()
            ",
            actor.id,
            sqlx::types::Json(preferences) as sqlx::types::Json<&Preferences>,
        )
        .execute(&self.pool)
        .await?;
        if stored.rows_affected() == 0 {
            return Err(IdentityError::Unauthorized);
        }
        Ok(())
    }
}
