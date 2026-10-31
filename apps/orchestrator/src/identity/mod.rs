//! Human accounts and current database-backed session authority.

pub mod access;
mod accounts;
pub mod oidc;
pub use accounts::AccountSummary;
mod commands;
pub mod teams;
pub use commands::{IdentityEvent, Receipt};
mod invitations;
pub use invitations::{Invitation, InvitationSummary};
mod passwords;
mod sessions;

use sqlx::PgPool;
use uuid::Uuid;

#[derive(Clone)]
pub struct Identity {
    pool: PgPool,
    pub(crate) browser_origin: Option<String>,
}

#[derive(Debug)]
pub enum IdentityError {
    InvalidInput,
    /// `InvalidInput` with the error that caused it.
    Invalid(Source),
    Unauthorized,
    /// `Unauthorized` with the error that caused it; clients see the same response.
    Rejected(Source),
    Forbidden,
    Conflict,
    NotFound,
    Database(sqlx::Error),
    Password(Source),
    Busy,
    OidcUnavailable(Source),
    /// `None` when no usable credential exists rather than one failing to open.
    CredentialUnavailable(Option<Source>),
}

pub type Source = Box<dyn std::error::Error + Send + Sync>;

impl std::fmt::Display for IdentityError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::InvalidInput | Self::Invalid(_) => "invalid input",
            Self::Unauthorized | Self::Rejected(_) => "unauthorized",
            Self::Forbidden => "forbidden",
            Self::Conflict => "conflict",
            Self::NotFound => "not found",
            Self::Database(_) | Self::Password(_) => "identity unavailable",
            Self::Busy => "identity busy",
            Self::OidcUnavailable(_) => "OIDC unavailable",
            Self::CredentialUnavailable(_) => "credential unavailable",
        })
    }
}

impl std::error::Error for IdentityError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Database(error) => Some(error),
            Self::Invalid(error)
            | Self::Rejected(error)
            | Self::Password(error)
            | Self::OidcUnavailable(error) => Some(error.as_ref()),
            Self::CredentialUnavailable(error) => error.as_deref().map(|error| error as _),
            _ => None,
        }
    }
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
        Self {
            pool,
            browser_origin: None,
        }
    }

    async fn require_administrator(&self, actor: &Actor) -> Result<(), IdentityError> {
        let allowed = sqlx::query_scalar!(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM human_accounts
                WHERE id = $1 AND active AND role = 'administrator'
            ) AS "allowed!"
            "#,
            actor.id,
        )
        .fetch_one(&self.pool)
        .await?;
        if !allowed {
            return Err(IdentityError::Forbidden);
        }
        Ok(())
    }
}
