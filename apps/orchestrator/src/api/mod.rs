//! HTTP entry point for the Cubebot control plane.
//!
//! Database-enabled mode exposes the initial authenticated identity endpoints.

mod access;
mod agents;
mod browser;
mod connections;
mod identity;
mod invitations;
mod oidc;
mod runtime;
mod work;

use axum::{Json, Router, routing::get};
use cubebot_protocol::{Liveness, LivenessStatus};

pub fn router() -> Router {
    Router::new().route("/api/health/live", get(liveness))
}

async fn liveness() -> Json<Liveness> {
    Json(Liveness {
        status: LivenessStatus::Alive,
    })
}

/// The browser API. `just api` writes it for the web client's generated types.
pub fn openapi() -> String {
    use utoipa::OpenApi;
    let mut doc = identity::Doc::openapi();
    doc.merge(agents::Doc::openapi());
    doc.merge(connections::Doc::openapi());
    doc.merge(work::Doc::openapi());
    doc.to_pretty_json().unwrap_or_default()
}

/// Configured application endpoints share one concrete PostgreSQL pool.
pub fn database_router(pool: sqlx::PgPool) -> Router {
    identity_router(pool.clone(), crate::identity::Identity::new(pool))
}

fn identity_router(pool: sqlx::PgPool, identity: crate::identity::Identity) -> Router {
    router()
        .merge(agents::router(pool.clone(), identity.clone()))
        .merge(work::router(pool.clone(), identity.clone()))
        .merge(identity::router(identity.clone()))
        .merge(invitations::router(identity.clone()))
        .merge(access::router(identity))
        .route("/api/health/ready", get(readiness).with_state(pool))
}

pub async fn configured_router(
    pool: sqlx::PgPool,
    origin: Option<String>,
    config: Option<(crate::identity::oidc::Config, Option<String>)>,
) -> Result<Router, crate::identity::IdentityError> {
    let credentials = std::env::var_os("CUBEBOT_CREDENTIAL_KEY_FILE")
        .map(|path| crate::connections::Credentials::from_file(std::path::Path::new(&path)))
        .transpose()
        .map_err(|_| crate::identity::IdentityError::CredentialUnavailable)?;
    configured_router_with_credentials(pool, origin, config, credentials).await
}

pub async fn configured_router_with_credentials(
    pool: sqlx::PgPool,
    origin: Option<String>,
    config: Option<(crate::identity::oidc::Config, Option<String>)>,
    credentials: Option<crate::connections::Credentials>,
) -> Result<Router, crate::identity::IdentityError> {
    let mut identity = crate::identity::Identity::new(pool.clone());
    if let Some(origin) = origin {
        identity.browser_origin = Some(crate::identity::oidc::Config::public_origin(&origin)?);
    }
    let mut routes = if let Some((config, secret)) = config {
        if identity.browser_origin.is_some() && identity.browser_origin != Some(config.origin()?) {
            return Err(crate::identity::IdentityError::InvalidInput);
        }
        let oidc = crate::identity::oidc::Oidc::configure(identity, config, secret).await?;
        identity = oidc.identity.clone();
        identity_router(pool.clone(), identity.clone()).merge(self::oidc::router(oidc))
    } else {
        crate::identity::oidc::disable(&pool).await?;
        identity_router(pool.clone(), identity.clone())
    };
    if let Some(credentials) = credentials {
        routes = routes.merge(connections::router(
            identity.clone(),
            crate::connections::Connections::new(pool, credentials),
        ));
    }
    Ok(routes.layer(axum::middleware::from_fn_with_state(
        identity,
        browser::csrf,
    )))
}

async fn readiness(
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
) -> axum::http::StatusCode {
    match sqlx::query("SELECT 1").execute(&pool).await {
        Ok(_) => axum::http::StatusCode::OK,
        Err(_) => axum::http::StatusCode::SERVICE_UNAVAILABLE,
    }
}

/// Explicit runtime routes. Normal startup does not discover a Kubernetes configuration.
pub fn runtime_router(identity: crate::agents::runtime::RuntimeIdentity) -> Router {
    runtime::router(identity)
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_committed_browser_api_document_is_current() {
        let committed: serde_json::Value =
            serde_json::from_str(include_str!("../../../web/src/api/openapi.json")).unwrap();
        let current: serde_json::Value = serde_json::from_str(&super::openapi()).unwrap();
        assert!(committed == current, "run `just api` and commit the result");
    }
}
