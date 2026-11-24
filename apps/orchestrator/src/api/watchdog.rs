use super::Installation;
use crate::{
    identity::{Identity, IdentityError},
    watchdog::{Dismiss, Watchdog},
};
use axum::{
    Extension, Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, put},
};
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    watchdog: Watchdog,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(overview, dismiss, unmute))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/watchdog", get(overview))
        .route("/api/watchdog/findings/{finding}/dismiss", put(dismiss))
        .route("/api/watchdog/mutes/{mute}", delete(unmute))
        .with_state(Api {
            identity,
            watchdog: Watchdog::new(pool),
        })
}

/// Open and recently dismissed findings, mutes and posture checks; administrators only.
#[utoipa::path(get, path = "/api/watchdog", operation_id = "watchdog", responses((status = 200, body = crate::watchdog::Overview)))]
async fn overview(
    State(api): State<Api>,
    headers: HeaderMap,
    flags: Option<Extension<Installation>>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let flags = flags.map_or_else(Installation::default, |Extension(flags)| flags);
    Ok((
        [("cache-control", "no-store")],
        Json(api.watchdog.overview(&actor, &flags).await?),
    ))
}

#[utoipa::path(put, path = "/api/watchdog/findings/{finding}/dismiss", operation_id = "dismiss_finding", params(("finding" = Uuid, Path)), request_body = Dismiss, responses((status = 204), (status = 404)))]
async fn dismiss(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(finding): Path<Uuid>,
    Json(input): Json<Dismiss>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.watchdog.dismiss(&actor, finding, &input).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(delete, path = "/api/watchdog/mutes/{mute}", operation_id = "unmute_rule", params(("mute" = Uuid, Path)), responses((status = 204), (status = 404)))]
async fn unmute(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(mute): Path<Uuid>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.watchdog.unmute(&actor, mute).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Clone)]
struct Watched {
    pool: sqlx::PgPool,
    identity: Identity,
    refusals: crate::watchdog::platform::Refusals,
}

/// Counts refused requests per session by the digest of its token, and opens a finding for a
/// burst; requests that are not refused only have their credential header read.
pub fn watched(router: Router, pool: sqlx::PgPool, origin: Option<String>) -> Router {
    let mut identity = Identity::new(pool.clone());
    identity.browser_origin = origin;
    let watched = Watched {
        pool,
        identity,
        refusals: crate::watchdog::platform::Refusals::default(),
    };
    router.layer(axum::middleware::from_fn_with_state(watched, refused))
}

async fn refused(
    State(watched): State<Watched>,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let credential = super::browser::credential(&watched.identity, request.headers())
        .ok()
        .map(crate::identity::digest);
    let response = next.run(request).await;
    if response.status() == StatusCode::FORBIDDEN
        && let Some(digest) = credential
        && watched.refusals.refused(digest.clone())
    {
        tokio::spawn(report(watched.pool, digest));
    }
    response
}

/// Opens the burst's finding; a failure is logged.
async fn report(pool: sqlx::PgPool, digest: Vec<u8>) {
    let Some((hit, agent)) = named(&pool, digest).await else {
        return;
    };
    if let Err(error) = crate::watchdog::open(&pool, &hit, agent, None, 0).await {
        tracing::warn!(?error, "watchdog finding not recorded");
    }
}

/// The person or agent whose session the burst came from; an ended session is let go.
async fn named(
    pool: &sqlx::PgPool,
    digest: Vec<u8>,
) -> Option<(crate::watchdog::rules::Hit, Option<Uuid>)> {
    let row = sqlx::query!(
        r#"
        SELECT (SELECT a.login FROM human_sessions s JOIN human_accounts a ON a.id = s.account_id
                WHERE s.token_hash = $1) AS "login?",
               (SELECT r.agent_id FROM runtime_session_access r WHERE r.token_hash = $1 LIMIT 1) AS "agent?"
        "#,
        digest,
    )
    .fetch_one(pool)
    .await
    .inspect_err(|error| tracing::warn!(?error, "the refused session could not be named"))
    .ok()?;
    match (row.login, row.agent) {
        (Some(login), _) => Some((crate::watchdog::platform::refusal(Some(&login)), None)),
        (None, Some(agent)) => Some((crate::watchdog::platform::refusal(None), Some(agent))),
        (None, None) => None,
    }
}
