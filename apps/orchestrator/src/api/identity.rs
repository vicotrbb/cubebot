use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::identity::{Actor, Identity, IdentityError};

pub fn router(identity: Identity) -> Router {
    Router::new()
        .route("/api/identity/login", post(login))
        .route("/api/identity/logout", post(logout))
        .route("/api/identity/me", get(me))
        .route("/api/identity/accounts", post(create_account))
        .route("/api/identity/suspend", post(suspend))
        .with_state(identity)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Login {
    login: String,
    password: String,
}

#[derive(Serialize)]
struct Token {
    token: String,
}

async fn login(
    State(identity): State<Identity>,
    Json(input): Json<Login>,
) -> Result<impl IntoResponse, IdentityError> {
    let session = identity.login(&input.login, &input.password).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(Token {
            token: session.token,
        }),
    ))
}

fn bearer(headers: &HeaderMap) -> Result<&str, IdentityError> {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(IdentityError::Unauthorized)
}

pub(super) async fn actor(
    identity: &Identity,
    headers: &HeaderMap,
) -> Result<Actor, IdentityError> {
    identity.authenticate(bearer(headers)?).await
}

#[derive(Serialize)]
struct Account {
    id: Uuid,
    administrator: bool,
}

async fn me(
    State(identity): State<Identity>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(Account {
            id: actor.id,
            administrator: actor.administrator,
        }),
    ))
}

async fn logout(
    State(identity): State<Identity>,
    headers: HeaderMap,
) -> Result<Json<bool>, IdentityError> {
    identity.logout(bearer(&headers)?).await?;
    Ok(Json(true))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateAccount {
    login: String,
    password: String,
    #[serde(default)]
    administrator: bool,
}

async fn create_account(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<CreateAccount>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    let id = identity
        .create_account(&actor, &input.login, &input.password, input.administrator)
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(Account {
            id,
            administrator: input.administrator,
        }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Subject {
    id: Uuid,
}

async fn suspend(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<Subject>,
) -> Result<Json<bool>, IdentityError> {
    identity
        .suspend(&actor(&identity, &headers).await?, input.id)
        .await?;
    Ok(Json(true))
}

#[derive(Serialize)]
struct ErrorBody {
    error: &'static str,
}

impl IntoResponse for IdentityError {
    fn into_response(self) -> Response {
        let (status, error) = match self {
            Self::InvalidInput => (StatusCode::BAD_REQUEST, "invalid_input"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::Conflict => (StatusCode::CONFLICT, "conflict"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Database(_) | Self::Password => {
                (StatusCode::SERVICE_UNAVAILABLE, "identity_unavailable")
            }
        };
        (
            status,
            [("cache-control", "no-store")],
            Json(ErrorBody { error }),
        )
            .into_response()
    }
}
