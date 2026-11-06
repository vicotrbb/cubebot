use axum::{
    Json, Router,
    body::Bytes,
    extract::{DefaultBodyLimit, Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::identity::{Actor, Identity, IdentityError, audit::Event};

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    login,
    setup_state,
    setup,
    me,
    logout,
    preferences,
    set_preferences,
    profile,
    set_profile,
    avatars,
    avatar,
    set_avatar,
    remove_avatar,
    accounts,
    create_account,
    suspend,
    oidc_bindings,
    revoke_oidc
))]
pub(super) struct Doc;

pub fn router(identity: Identity) -> Router {
    Router::new()
        .route("/api/identity/login", post(login))
        .route("/api/identity/setup", get(setup_state).post(setup))
        .route("/api/identity/oidc/bindings", get(oidc_bindings))
        .route("/api/identity/oidc/revoke", post(revoke_oidc))
        .route("/api/identity/logout", post(logout))
        .route("/api/identity/me", get(me))
        .route(
            "/api/identity/me/preferences",
            get(preferences).put(set_preferences),
        )
        .route("/api/identity/me/profile", get(profile).put(set_profile))
        .route("/api/identity/avatars", get(avatars))
        .route("/api/identity/accounts/{id}/avatar", get(avatar))
        .route(
            "/api/identity/me/avatar",
            put(set_avatar)
                .delete(remove_avatar)
                .layer(DefaultBodyLimit::max(crate::identity::AVATAR_BYTES)),
        )
        .route("/api/identity/accounts", get(accounts).post(create_account))
        .route("/api/identity/suspend", post(suspend))
        .with_state(identity)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AccountCursor {
    after: Option<Uuid>,
}

#[utoipa::path(get, path = "/api/identity/accounts", params(("after" = Option<Uuid>, Query)), responses((status = 200, body = Vec<crate::identity::AccountSummary>)))]
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

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct Login {
    login: String,
    password: String,
}

#[derive(Serialize, utoipa::ToSchema)]
struct Token {
    token: String,
}

#[utoipa::path(post, path = "/api/identity/login", request_body = Login, responses((status = 200, body = Token)))]
async fn login(
    State(identity): State<Identity>,
    Json(input): Json<Login>,
) -> Result<impl IntoResponse, IdentityError> {
    let session = match identity.login(&input.login, &input.password).await {
        Err(IdentityError::Unauthorized) => {
            // Anyone can post this, so what it stores stays bounded; no real login is longer.
            let login: String = input.login.chars().take(64).collect();
            let event = Event::anonymous("session.sign_in_failed")
                .detail(serde_json::json!({"login": login, "method": "password"}));
            identity.audit(event).await;
            return Err(IdentityError::Unauthorized);
        }
        session => session?,
    };
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

#[derive(Serialize, utoipa::ToSchema)]
struct Setup {
    open: bool,
}

#[utoipa::path(get, path = "/api/identity/setup", responses((status = 200, body = Setup)))]
async fn setup_state(State(identity): State<Identity>) -> Result<impl IntoResponse, IdentityError> {
    Ok((
        [("cache-control", "no-store")],
        Json(Setup {
            open: identity.setup_open().await?,
        }),
    ))
}

/// Creates the installation's first account as its administrator and signs it in.
#[utoipa::path(post, path = "/api/identity/setup", request_body = Login, responses((status = 200, body = Token), (status = 409)))]
async fn setup(
    State(identity): State<Identity>,
    Json(input): Json<Login>,
) -> Result<impl IntoResponse, IdentityError> {
    if !identity.setup_open().await? {
        return Err(IdentityError::Conflict);
    }
    identity.bootstrap(&input.login, &input.password).await?;
    login(State(identity), Json(input)).await
}

pub(super) async fn actor(
    identity: &Identity,
    headers: &HeaderMap,
) -> Result<Actor, IdentityError> {
    identity
        .authenticate(super::browser::credential(identity, headers)?)
        .await
}

#[derive(Serialize, utoipa::ToSchema)]
struct Account {
    id: Uuid,
    login: String,
    /// A display name; accounts have none yet.
    name: Option<String>,
    administrator: bool,
}

#[utoipa::path(get, path = "/api/identity/me", responses((status = 200, body = Account)))]
async fn me(
    State(identity): State<Identity>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    let login = identity.account_login(&actor).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(Account {
            id: actor.id,
            login,
            // shortcut: no display name column; add one with OIDC profile claims when needed.
            name: None,
            administrator: actor.administrator,
        }),
    ))
}

#[utoipa::path(get, path = "/api/identity/me/preferences", responses((status = 200, body = crate::identity::Preferences)))]
async fn preferences(
    State(identity): State<Identity>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(identity.preferences(&actor).await?),
    ))
}

/// Replaces the whole document; the last write wins.
#[utoipa::path(put, path = "/api/identity/me/preferences", request_body = crate::identity::Preferences, responses((status = 200, body = crate::identity::Preferences)))]
async fn set_preferences(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<crate::identity::Preferences>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    identity.set_preferences(&actor, &input).await?;
    Ok(([("cache-control", "no-store")], Json(input)))
}

/// What the caller tells agents about themselves.
#[utoipa::path(get, path = "/api/identity/me/profile", responses((status = 200, body = crate::identity::profiles::Profile)))]
async fn profile(
    State(identity): State<Identity>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(identity.profile(&actor).await?),
    ))
}

/// Replaces the caller's whole profile; agents that person reaches read it.
#[utoipa::path(put, path = "/api/identity/me/profile", request_body = crate::identity::profiles::Profile, responses((status = 200, body = crate::identity::profiles::Profile), (status = 400)))]
async fn set_profile(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<crate::identity::profiles::Profile>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(identity.set_profile(&actor, &input).await?),
    ))
}

/// Everyone's picture revision, for people who have one.
#[utoipa::path(get, path = "/api/identity/avatars", responses((status = 200, body = Vec<crate::identity::AvatarRevision>)))]
async fn avatars(
    State(identity): State<Identity>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    actor(&identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(identity.avatars().await?),
    ))
}

/// A person's picture. Pages ask with `?v=<revision>`, so a new picture is a new URL and this
/// one may be cached.
#[utoipa::path(get, path = "/api/identity/accounts/{id}/avatar", params(("id" = Uuid, Path)), responses((status = 200, content_type = "image/png"), (status = 404)))]
async fn avatar(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, IdentityError> {
    actor(&identity, &headers).await?;
    let image = identity.avatar(id).await?.ok_or(IdentityError::NotFound)?;
    let content_type = crate::agents::content_type(&image).ok_or(IdentityError::NotFound)?;
    Ok((
        [
            ("cache-control", "private, max-age=31536000, immutable"),
            ("content-type", content_type),
        ],
        image,
    ))
}

/// Sets the caller's picture from a JPEG or PNG body of at most 1 MiB; returns its revision.
#[utoipa::path(put, path = "/api/identity/me/avatar", request_body(content = Vec<u8>, content_type = "image/png"), responses((status = 200, body = i64), (status = 400)))]
async fn set_avatar(
    State(identity): State<Identity>,
    headers: HeaderMap,
    image: Bytes,
) -> Result<Json<i64>, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(Json(identity.set_avatar(&actor, Some(&image)).await?))
}

#[utoipa::path(delete, path = "/api/identity/me/avatar", responses((status = 200, body = i64)))]
async fn remove_avatar(
    State(identity): State<Identity>,
    headers: HeaderMap,
) -> Result<Json<i64>, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(Json(identity.set_avatar(&actor, None).await?))
}

#[utoipa::path(post, path = "/api/identity/logout", responses((status = 200, body = bool)))]
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

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct CreateAccount {
    request_id: Uuid,
    login: String,
    password: String,
    #[serde(default)]
    administrator: bool,
}

#[utoipa::path(post, path = "/api/identity/accounts", request_body = CreateAccount, responses((status = 201, body = Account)))]
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
            login: input.login,
            name: None,
            administrator: input.administrator,
        }),
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct Subject {
    request_id: Uuid,
    expected_revision: i64,
    id: Uuid,
}

#[utoipa::path(post, path = "/api/identity/suspend", request_body = Subject, responses((status = 200, body = crate::identity::Receipt)))]
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
            Self::InvalidInput | Self::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_input"),
            Self::Unauthorized | Self::Rejected(_) => (StatusCode::UNAUTHORIZED, "unauthorized"),
            Self::Forbidden => (StatusCode::FORBIDDEN, "forbidden"),
            Self::Conflict => (StatusCode::CONFLICT, "conflict"),
            Self::Busy => (StatusCode::TOO_MANY_REQUESTS, "identity_busy"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::CredentialUnavailable(_) => {
                (StatusCode::SERVICE_UNAVAILABLE, "credential_unavailable")
            }
            Self::OidcUnavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "oidc_unavailable"),
            Self::Database(_) | Self::Password(_) => {
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

#[utoipa::path(get, path = "/api/identity/oidc/bindings", params(("after" = Option<Uuid>, Query)), responses((status = 200, body = Vec<crate::identity::oidc::Binding>)))]
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

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct RevokeOidc {
    id: Uuid,
    expected_revision: i64,
    request_id: Uuid,
}

#[utoipa::path(post, path = "/api/identity/oidc/revoke", request_body = RevokeOidc, responses((status = 200, body = crate::identity::Receipt)))]
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
