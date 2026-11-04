//! HTTP entry point for the Cubebot control plane.

mod access;
mod activity;
mod agents;
mod browser;
mod connections;
mod desktop;
mod files;
mod identity;
mod installation;
mod invitations;
mod mcp;
mod memories;
mod oidc;
mod rooms;
mod routines;
mod runtime;
mod skills;
mod vault;
mod work;

use axum::{Json, Router, routing::get};
use cubebot_protocol::{Liveness, LivenessStatus};
pub use installation::Installation;

#[derive(utoipa::OpenApi)]
#[openapi(paths(liveness, readiness))]
struct HealthDoc;

pub fn router() -> Router {
    Router::new().route("/api/health/live", get(liveness))
}

/// One span per request, named by method and matched route template; never the raw path,
/// query or headers.
pub fn traced(router: Router) -> Router {
    router.layer(axum::middleware::from_fn(request_span))
}

async fn request_span(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use tracing::Instrument;
    let route = request
        .extensions()
        .get::<axum::extract::MatchedPath>()
        .map_or("unmatched", |path| path.as_str());
    let span = tracing::info_span!("http.request", method = %request.method(), route);
    next.run(request).instrument(span).await
}

/// Process liveness only.
#[utoipa::path(get, path = "/api/health/live", responses((status = 200, body = Liveness)))]
async fn liveness() -> Json<Liveness> {
    Json(Liveness {
        status: LivenessStatus::Alive,
    })
}

/// The browser API. `just api` writes it for the web client's generated types.
pub fn openapi() -> String {
    use utoipa::OpenApi;
    let mut doc = identity::Doc::openapi();
    doc.merge(HealthDoc::openapi());
    doc.merge(invitations::Doc::openapi());
    doc.merge(access::Doc::openapi());
    doc.merge(oidc::Doc::openapi());
    doc.merge(installation::Doc::openapi());
    doc.merge(activity::Doc::openapi());
    doc.merge(agents::Doc::openapi());
    doc.merge(connections::Doc::openapi());
    doc.merge(work::Doc::openapi());
    doc.merge(desktop::Doc::openapi());
    doc.merge(rooms::Doc::openapi());
    doc.merge(memories::Doc::openapi());
    doc.merge(skills::Doc::openapi());
    doc.merge(vault::Doc::openapi());
    doc.merge(mcp::Doc::openapi());
    doc.merge(files::Doc::openapi());
    doc.merge(routines::Doc::openapi());
    doc.to_pretty_json().unwrap_or_default()
}

/// A refusal: 409 with the reason the model or person reads.
fn refused(reason: &str) -> axum::response::Response {
    use axum::response::IntoResponse;
    let body = Json(serde_json::json!({ "error": reason }));
    (axum::http::StatusCode::CONFLICT, body).into_response()
}

/// The credential of an `Authorization: Bearer` header.
fn bearer(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

/// Configured application endpoints share one concrete PostgreSQL pool.
pub fn identity_router(pool: sqlx::PgPool, identity: crate::identity::Identity) -> Router {
    router()
        .merge(agents::router(pool.clone(), identity.clone()))
        .merge(installation::router(pool.clone(), identity.clone()))
        .merge(activity::router(pool.clone(), identity.clone()))
        .merge(desktop::router(pool.clone(), identity.clone()))
        .merge(work::router(pool.clone(), identity.clone()))
        .merge(rooms::router(pool.clone(), identity.clone()))
        .merge(routines::router(pool.clone(), identity.clone()))
        .merge(memories::router(pool.clone(), identity.clone()))
        .merge(skills::router(pool.clone(), identity.clone()))
        .merge(files::router(identity.clone()))
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
        .map_err(|error| {
            crate::identity::IdentityError::CredentialUnavailable(Some(error.into()))
        })?;
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
        let connections = crate::connections::Connections::new(pool, credentials);
        routes = routes
            .merge(connections::router(identity.clone(), connections.clone()))
            .merge(vault::router(identity.clone(), connections.clone()))
            .merge(mcp::router(identity.clone(), connections));
    }
    Ok(routes.layer(axum::middleware::from_fn_with_state(
        identity,
        browser::csrf,
    )))
}

/// Starts an event stream's first frames with a comment. Firefox reports an EventSource open
/// only once body bytes arrive, and the console loads history on open, so a quiet stream would
/// otherwise keep it waiting for the first keep-alive, 15 s later.
fn opening(mut first: Vec<axum::response::sse::Event>) -> Vec<axum::response::sse::Event> {
    first.insert(0, axum::response::sse::Event::default().comment("open"));
    first
}

/// The configured database answers; 503 otherwise.
#[utoipa::path(get, path = "/api/health/ready", responses((status = 200), (status = 503)))]
async fn readiness(
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
) -> axum::http::StatusCode {
    match sqlx::query!("SELECT 1 AS one").fetch_one(&pool).await {
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
