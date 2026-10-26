use crate::identity::{
    IdentityError,
    oidc::{Oidc, Started},
};
use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, HeaderValue},
    response::{IntoResponse, Redirect},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};

pub(super) fn router(oidc: Oidc) -> Router {
    Router::new()
        .route("/api/identity/oidc/start", post(start))
        .route("/api/identity/oidc/link", post(link))
        .route("/api/identity/oidc/callback", get(callback))
        .with_state(oidc)
}

#[derive(Serialize)]
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

async fn start(State(oidc): State<Oidc>) -> Result<impl IntoResponse, IdentityError> {
    started(&oidc, oidc.start(None).await?)
}

async fn link(
    State(oidc): State<Oidc>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let token = super::browser::credential(&oidc.identity, &headers)?;
    started(&oidc, oidc.start(Some(token)).await?)
}

#[derive(Deserialize)]
struct Callback {
    state: String,
    code: String,
}

async fn callback(
    State(oidc): State<Oidc>,
    headers: HeaderMap,
    Query(input): Query<Callback>,
) -> Result<impl IntoResponse, IdentityError> {
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
