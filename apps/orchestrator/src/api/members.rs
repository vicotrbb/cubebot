//! Roles, ownership, sharing directory and a person's own sign-in security.

use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{delete, get, post},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::identity::{
    Identity, IdentityError, Receipt,
    members::{ChangeRole, HandOver, Reactivate, SavePolicy, Transfer, TransferAll},
    security::{AccountRequest, NewToken},
};

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    standing,
    directory,
    policy,
    save_policy,
    change_role,
    reactivate,
    transfer,
    transfer_all,
    hand_over,
    resource_access,
    change_password,
    sessions,
    end_session,
    create_token,
    enroll_two_factor,
    confirm_two_factor,
    disable_two_factor,
    reset_two_factor,
    create_password_reset,
    redeem_password_reset,
    inventory,
    spend
))]
pub(super) struct Doc;

pub fn router(identity: Identity) -> Router {
    Router::new()
        .route("/api/identity/me/standing", get(standing))
        .route("/api/identity/directory", get(directory))
        .route("/api/identity/policy", get(policy).put(save_policy))
        .route("/api/identity/accounts/role", post(change_role))
        .route("/api/identity/reactivate", post(reactivate))
        .route("/api/identity/ownership/transfer", post(transfer))
        .route("/api/identity/ownership/transfer-all", post(transfer_all))
        .route("/api/identity/installation/hand-over", post(hand_over))
        .route("/api/identity/access/resources/{id}", get(resource_access))
        .route("/api/identity/me/password", post(change_password))
        .route("/api/identity/me/sessions", get(sessions))
        .route("/api/identity/me/sessions/{id}", delete(end_session))
        .route("/api/identity/me/tokens", post(create_token))
        .route("/api/identity/me/two-factor", post(enroll_two_factor))
        .route(
            "/api/identity/me/two-factor/confirm",
            post(confirm_two_factor),
        )
        .route(
            "/api/identity/me/two-factor/disable",
            post(disable_two_factor),
        )
        .route(
            "/api/identity/accounts/two-factor-reset",
            post(reset_two_factor),
        )
        .route(
            "/api/identity/accounts/password-reset",
            post(create_password_reset),
        )
        .route("/api/identity/password-reset", post(redeem_password_reset))
        .route("/api/identity/inventory", get(inventory))
        .route("/api/identity/spend", get(spend))
        .with_state(identity)
}

fn no_store<T: Serialize>(value: T) -> impl IntoResponse {
    ([("cache-control", "no-store")], Json(value))
}

async fn actor(
    identity: &Identity,
    headers: &HeaderMap,
) -> Result<crate::identity::Actor, IdentityError> {
    super::identity::actor(identity, headers).await
}

#[utoipa::path(get, path = "/api/identity/me/standing", responses((status = 200, body = crate::identity::members::Standing)))]
async fn standing(
    State(identity): State<Identity>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(no_store(identity.standing(&actor).await?))
}

#[utoipa::path(get, path = "/api/identity/directory", responses((status = 200, body = crate::identity::members::Directory)))]
async fn directory(
    State(identity): State<Identity>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(no_store(identity.directory(&actor).await?))
}

#[utoipa::path(get, path = "/api/identity/policy", responses((status = 200, body = crate::identity::Policy)))]
async fn policy(
    State(identity): State<Identity>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(no_store(identity.policy(&actor).await?))
}

#[utoipa::path(put, path = "/api/identity/policy", request_body = SavePolicy, responses((status = 200, body = Receipt)))]
async fn save_policy(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<SavePolicy>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(no_store(identity.save_policy(&actor, &input).await?))
}

#[utoipa::path(post, path = "/api/identity/accounts/role", request_body = ChangeRole, responses((status = 200, body = Receipt)))]
async fn change_role(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<ChangeRole>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(no_store(identity.change_role(&actor, &input).await?))
}

#[utoipa::path(post, path = "/api/identity/reactivate", request_body = Reactivate, responses((status = 200, body = Receipt)))]
async fn reactivate(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<Reactivate>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(no_store(identity.reactivate(&actor, &input).await?))
}

#[utoipa::path(post, path = "/api/identity/ownership/transfer", request_body = Transfer, responses((status = 200, body = Receipt)))]
async fn transfer(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<Transfer>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(no_store(identity.transfer(&actor, &input).await?))
}

#[utoipa::path(post, path = "/api/identity/ownership/transfer-all", request_body = TransferAll, responses((status = 200, body = Receipt)))]
async fn transfer_all(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<TransferAll>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(no_store(identity.transfer_all(&actor, &input).await?))
}

#[utoipa::path(post, path = "/api/identity/installation/hand-over", request_body = HandOver, responses((status = 200, body = Receipt)))]
async fn hand_over(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<HandOver>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(no_store(identity.hand_over(&actor, &input).await?))
}

/// The owner and the grants of one agent or room, for those who manage it.
#[utoipa::path(get, path = "/api/identity/access/resources/{id}", params(("id" = Uuid, Path)), responses((status = 200, body = crate::identity::access::ResourceAccess)))]
async fn resource_access(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(no_store(identity.resource_access(&actor, id).await?))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct ChangePassword {
    current: String,
    /// The new password.
    password: String,
}

/// Ends every other session and personal token.
#[utoipa::path(post, path = "/api/identity/me/password", request_body = ChangePassword, responses((status = 204), (status = 400)))]
async fn change_password(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<ChangePassword>,
) -> Result<impl IntoResponse, IdentityError> {
    let token = super::browser::credential(&identity, &headers)?;
    let actor = identity.authenticate(token).await?;
    identity
        .change_password(&actor, &input.current, &input.password, token)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(get, path = "/api/identity/me/sessions", responses((status = 200, body = Vec<crate::identity::security::SessionSummary>)))]
async fn sessions(
    State(identity): State<Identity>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let token = super::browser::credential(&identity, &headers)?;
    let actor = identity.authenticate(token).await?;
    Ok(no_store(identity.sessions(&actor, token).await?))
}

#[utoipa::path(delete, path = "/api/identity/me/sessions/{id}", params(("id" = Uuid, Path)), responses((status = 204)))]
async fn end_session(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    identity.end_session(&actor, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, utoipa::ToSchema)]
struct Secret {
    token: String,
}

/// The token is in this response only.
#[utoipa::path(post, path = "/api/identity/me/tokens", request_body = NewToken, responses((status = 201, body = Secret)))]
async fn create_token(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<NewToken>,
) -> Result<impl IntoResponse, IdentityError> {
    let caller = super::browser::credential(&identity, &headers)?;
    let actor = identity.authenticate(caller).await?;
    let token = identity.create_token(&actor, &input, caller).await?;
    Ok((StatusCode::CREATED, no_store(Secret { token })))
}

#[utoipa::path(post, path = "/api/identity/me/two-factor", responses((status = 200, body = crate::identity::security::Enrollment), (status = 409)))]
async fn enroll_two_factor(
    State(identity): State<Identity>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(no_store(identity.enroll_two_factor(&actor).await?))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct Code {
    code: String,
}

#[derive(Serialize, utoipa::ToSchema)]
struct RecoveryCodes {
    /// Single-use codes for when the authenticator is lost; shown this once.
    codes: Vec<String>,
}

#[utoipa::path(post, path = "/api/identity/me/two-factor/confirm", request_body = Code, responses((status = 200, body = RecoveryCodes), (status = 400)))]
async fn confirm_two_factor(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<Code>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    let codes = identity.confirm_two_factor(&actor, &input.code).await?;
    Ok(no_store(RecoveryCodes { codes }))
}

#[utoipa::path(post, path = "/api/identity/me/two-factor/disable", request_body = Code, responses((status = 204), (status = 400)))]
async fn disable_two_factor(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<Code>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    identity.disable_two_factor(&actor, &input.code).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(post, path = "/api/identity/accounts/two-factor-reset", request_body = AccountRequest, responses((status = 200, body = Receipt)))]
async fn reset_two_factor(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<AccountRequest>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(no_store(identity.reset_two_factor(&actor, &input).await?))
}

/// A one-day link token for setting a new password; in this response only.
#[utoipa::path(post, path = "/api/identity/accounts/password-reset", request_body = AccountRequest, responses((status = 201, body = Secret)))]
async fn create_password_reset(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<AccountRequest>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    let token = identity.create_password_reset(&actor, &input).await?;
    Ok((StatusCode::CREATED, no_store(Secret { token })))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct Redeem {
    token: String,
    password: String,
}

#[utoipa::path(post, path = "/api/identity/password-reset", request_body = Redeem, responses((status = 204), (status = 401)))]
async fn redeem_password_reset(
    State(identity): State<Identity>,
    Json(input): Json<Redeem>,
) -> Result<impl IntoResponse, IdentityError> {
    identity
        .redeem_password_reset(&input.token, &input.password)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Every agent, room, team and connection with its owner; administrators and auditors.
#[utoipa::path(get, path = "/api/identity/inventory", responses((status = 200, body = Vec<crate::identity::oversight::OwnedResource>)))]
async fn inventory(
    State(identity): State<Identity>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    Ok(no_store(identity.inventory(&actor).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Period {
    days: Option<i32>,
}

/// Model spend per person over the last `days` (30 when left out).
#[utoipa::path(get, path = "/api/identity/spend", params(("days" = Option<i32>, Query)), responses((status = 200, body = Vec<crate::identity::oversight::OwnerSpend>)))]
async fn spend(
    State(identity): State<Identity>,
    headers: HeaderMap,
    axum::extract::Query(period): axum::extract::Query<Period>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = actor(&identity, &headers).await?;
    let days = period.days.unwrap_or(30);
    Ok(no_store(identity.spend_by_owner(&actor, days).await?))
}
