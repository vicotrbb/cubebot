use crate::identity::{
    IdentityError,
    audit::Event,
    oidc::{Oidc, SaveSingleSignOn, SingleSignOn, SingleSignOnSettings, Started},
};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

#[derive(utoipa::OpenApi)]
#[openapi(paths(start, link, status, settings, save, disable))]
pub(super) struct Doc;

pub(super) fn router(sso: SingleSignOn) -> Router {
    Router::new()
        .route("/api/identity/oidc", get(status))
        .route(
            "/api/identity/oidc/settings",
            get(settings).put(save).delete(disable),
        )
        .route("/api/identity/oidc/start", post(start))
        .route("/api/identity/oidc/link", post(link))
        .route("/api/identity/oidc/callback", get(callback))
        .with_state(sso)
}

/// 404 while single sign-on is off, as when its routes did not exist.
async fn current(sso: &SingleSignOn) -> Result<Oidc, IdentityError> {
    sso.current().await.ok_or(IdentityError::NotFound)
}

#[derive(Serialize, utoipa::ToSchema)]
struct Status {
    enabled: bool,
}

/// Whether the sign-in page offers single sign-on; needs no session.
#[utoipa::path(get, path = "/api/identity/oidc", operation_id = "single_sign_on_status", responses((status = 200, body = Status)))]
async fn status(State(sso): State<SingleSignOn>) -> impl IntoResponse {
    let enabled = sso.current().await.is_some();
    ([("cache-control", "no-store")], Json(Status { enabled }))
}

async fn administrator(
    sso: &SingleSignOn,
    headers: &HeaderMap,
) -> Result<uuid::Uuid, IdentityError> {
    let actor = super::identity::actor(&sso.identity(), headers).await?;
    if !actor.administrator {
        return Err(IdentityError::Forbidden);
    }
    Ok(actor.id)
}

/// Administrators only.
#[utoipa::path(get, path = "/api/identity/oidc/settings", operation_id = "single_sign_on_settings", responses((status = 200, body = SingleSignOnSettings)))]
async fn settings(
    State(sso): State<SingleSignOn>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    administrator(&sso, &headers).await?;
    Ok(([("cache-control", "no-store")], Json(sso.settings().await?)))
}

/// Administrators only. Checks the issuer before saving and applies at once; a changed provider
/// ends the sessions the previous one started. 409 when the deployment sets single sign-on,
/// 503 when the issuer's discovery fails.
#[utoipa::path(put, path = "/api/identity/oidc/settings", operation_id = "save_single_sign_on", request_body = SaveSingleSignOn, responses((status = 200, body = SingleSignOnSettings), (status = 409), (status = 503)))]
async fn save(
    State(sso): State<SingleSignOn>,
    headers: HeaderMap,
    Json(input): Json<SaveSingleSignOn>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = administrator(&sso, &headers).await?;
    let secret_changed = input.client_secret.is_some();
    let saved = sso.save(input).await?;
    let detail = serde_json::json!({
        "issuer": saved.issuer,
        "client_id": saved.client_id,
        "secret_changed": secret_changed,
    });
    sso.identity()
        .audit(Event::by(actor, "oidc.configure").detail(detail))
        .await;
    Ok(([("cache-control", "no-store")], Json(saved)))
}

/// Administrators only; ends single sign-on and its sessions. 409 when the deployment sets it.
#[utoipa::path(delete, path = "/api/identity/oidc/settings", operation_id = "disable_single_sign_on", responses((status = 204), (status = 409)))]
async fn disable(
    State(sso): State<SingleSignOn>,
    headers: HeaderMap,
) -> Result<StatusCode, IdentityError> {
    let actor = administrator(&sso, &headers).await?;
    sso.disable().await?;
    sso.identity().audit(Event::by(actor, "oidc.disable")).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, utoipa::ToSchema)]
struct Authorization {
    authorization_url: String,
}

fn flow_name(oidc: &Oidc, state: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(state.as_bytes());
    let suffix: String = hash.iter().map(|byte| format!("{byte:02x}")).collect();
    let prefix = if oidc
        .identity
        .browser_origin
        .as_deref()
        .is_some_and(|origin| origin.starts_with("https://"))
    {
        "__Host-"
    } else {
        ""
    };
    format!("{prefix}cubebot_oidc_{suffix}")
}

fn started(oidc: &Oidc, flow: Started) -> Result<axum::response::Response, IdentityError> {
    let secure = if oidc
        .identity
        .browser_origin
        .as_deref()
        .is_some_and(|origin| origin.starts_with("https://"))
    {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!(
        "{}={}; HttpOnly; SameSite=Lax; Path=/; Max-Age=600{secure}",
        flow_name(oidc, &flow.state),
        flow.browser_secret
    );
    let mut response = (
        [
            ("cache-control", "no-store"),
            ("referrer-policy", "no-referrer"),
        ],
        Json(Authorization {
            authorization_url: flow.authorization_url,
        }),
    )
        .into_response();
    response.headers_mut().append(
        "set-cookie",
        HeaderValue::from_str(&cookie).map_err(|_| IdentityError::InvalidInput)?,
    );
    Ok(response)
}

/// Starts a sign-in; the browser follows `authorization_url`.
#[utoipa::path(post, path = "/api/identity/oidc/start", responses((status = 200, body = Authorization)))]
async fn start(State(sso): State<SingleSignOn>) -> Result<impl IntoResponse, IdentityError> {
    let oidc = current(&sso).await?;
    started(&oidc, oidc.start(None).await?)
}

/// Starts linking the signed-in account to the identity provider.
#[utoipa::path(post, path = "/api/identity/oidc/link", responses((status = 200, body = Authorization)))]
async fn link(
    State(sso): State<SingleSignOn>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let oidc = current(&sso).await?;
    let token = super::browser::credential(&oidc.identity, &headers)?;
    started(&oidc, oidc.start(Some(token)).await?)
}

#[derive(Deserialize)]
struct Callback {
    state: String,
    code: String,
}

async fn callback(
    State(sso): State<SingleSignOn>,
    headers: HeaderMap,
    Query(input): Query<Callback>,
) -> Result<impl IntoResponse, IdentityError> {
    let oidc = current(&sso).await?;
    let name = flow_name(&oidc, &input.state);
    let browser = super::browser::cookie(&headers, &name)?.to_owned();
    let session = oidc.finish(&input.state, browser, input.code).await?;
    let mut response = super::browser::set_session(
        Redirect::to("/").into_response(),
        &oidc.identity,
        &session.token,
        86400,
    );
    response
        .headers_mut()
        .insert("cache-control", HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    let secure = if name.starts_with("__Host-") {
        "; Secure"
    } else {
        ""
    };
    let cleared = format!("{name}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}");
    response.headers_mut().append(
        "set-cookie",
        HeaderValue::from_str(&cleared).map_err(|_| IdentityError::InvalidInput)?,
    );
    Ok(response)
}
