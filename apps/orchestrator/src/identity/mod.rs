//! Human accounts and current database-backed session authority.

mod accounts;
mod passwords;
mod sessions;

use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct Identity {
    pool: PgPool,
}

#[derive(Debug)]
pub enum IdentityError {
    InvalidInput,
    Unauthorized,
    Forbidden,
    Conflict,
    Database(sqlx::Error),
    Password,
}

impl From<sqlx::Error> for IdentityError {
    fn from(error: sqlx::Error) -> Self {
        match error.as_database_error() {
            Some(database) if database.is_unique_violation() => Self::Conflict,
            _ => Self::Database(error),
        }
    }
}

/// Produced from a live session, never deserialized from caller input.
#[derive(Debug)]
pub struct Actor {
    pub id: Uuid,
    pub administrator: bool,
}

pub struct Session {
    pub token: String,
}

impl Identity {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    async fn require_administrator(&self, actor: &Actor) -> Result<(), IdentityError> {
        let allowed: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM human_accounts WHERE id = $1 AND active AND role = 'administrator')",
        )
        .bind(actor.id)
        .fetch_one(&self.pool)
        .await?;
        if !allowed {
            return Err(IdentityError::Forbidden);
        }
        Ok(())
    }
}
