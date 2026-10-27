use axum::{
    Json, Router,
    extract::{Query, State},
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
        .route("/api/identity/oidc/bindings", get(oidc_bindings))
        .route("/api/identity/oidc/revoke", post(revoke_oidc))
        .route("/api/identity/logout", post(logout))
        .route("/api/identity/me", get(me))
        .route("/api/identity/accounts", get(accounts).post(create_account))
        .route("/api/identity/suspend", post(suspend))
        .with_state(identity)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountCursor {
    after: Option<Uuid>,
}

async fn accounts(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Query(cursor): Query<AccountCursor>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(identity.accounts(&actor, cursor.after).await?),
    ))
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
    let response = (
        [("cache-control", "no-store")],
        Json(Token {
            token: session.token.clone(),
        }),
    )
        .into_response();
    Ok(super::browser::set_session(
        response,
        &identity,
        &session.token,
        86400,
    ))
}

pub(super) async fn actor(
    identity: &Identity,
    headers: &HeaderMap,
) -> Result<Actor, IdentityError> {
    identity
        .authenticate(super::browser::credential(identity, headers)?)
        .await
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
) -> Result<impl IntoResponse, IdentityError> {
    identity
        .logout(super::browser::credential(&identity, &headers)?)
        .await?;
    Ok(super::browser::set_session(
        ([("cache-control", "no-store")], Json(true)).into_response(),
        &identity,
        "",
        0,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CreateAccount {
    request_id: Uuid,
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
        .create_account(
            &actor,
            &input.login,
            &input.password,
            input.administrator,
            input.request_id,
        )
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
    request_id: Uuid,
    expected_revision: i64,
    id: Uuid,
}

async fn suspend(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<Subject>,
) -> Result<Json<crate::identity::Receipt>, IdentityError> {
    let receipt = identity
        .suspend(
            &actor(&identity, &headers).await?,
            input.id,
            input.expected_revision,
            input.request_id,
        )
        .await?;
    Ok(Json(receipt))
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
            Self::Busy => (StatusCode::TOO_MANY_REQUESTS, "identity_busy"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::CredentialUnavailable => {
                (StatusCode::SERVICE_UNAVAILABLE, "credential_unavailable")
            }
            Self::OidcUnavailable => (StatusCode::SERVICE_UNAVAILABLE, "oidc_unavailable"),
            Self::Database(_) | Self::Password => {
                (StatusCode::SERVICE_UNAVAILABLE, "identity_unavailable")
            }
        };
        let mut response = (
            status,
            [("cache-control", "no-store")],
            Json(ErrorBody { error }),
        )
            .into_response();
        if status == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert("retry-after", axum::http::HeaderValue::from_static("1"));
        }
        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saturated_password_capacity_returns_a_retryable_response() {
        let response = IdentityError::Busy.into_response();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(response.headers()["retry-after"], "1");
        assert_eq!(response.headers()["cache-control"], "no-store");
    }
}

async fn oidc_bindings(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Query(cursor): Query<AccountCursor>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(identity.oidc_bindings(&actor, cursor.after).await?),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RevokeOidc {
    id: Uuid,
    expected_revision: i64,
    request_id: Uuid,
}

async fn revoke_oidc(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<RevokeOidc>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(
            identity
                .revoke_oidc_binding(&actor, input.id, input.expected_revision, input.request_id)
                .await?,
        ),
    ))
}
