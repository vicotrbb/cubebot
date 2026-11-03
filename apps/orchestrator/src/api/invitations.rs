use axum::{
    Json, Router,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::identity::{Identity, IdentityError, Receipt};

#[derive(utoipa::OpenApi)]
#[openapi(paths(list, invite, accept, revoke))]
pub(super) struct Doc;

pub fn router(identity: Identity) -> Router {
    Router::new()
        .route("/api/identity/invitations", get(list).post(invite))
        .route("/api/identity/invitations/accept", post(accept))
        .route("/api/identity/invitations/revoke", post(revoke))
        .with_state(identity)
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct Invite {
    request_id: Uuid,
    login: String,
}

/// The token is in the first response only.
#[utoipa::path(post, path = "/api/identity/invitations", request_body = Invite, responses((status = 201, body = crate::identity::Invitation)))]
async fn invite(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<Invite>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&identity, &headers).await?;
    let invitation = identity
        .invite(&actor, &input.login, input.request_id)
        .await?;
    Ok((
        StatusCode::CREATED,
        [("cache-control", "no-store")],
        Json(invitation),
    ))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct Accept {
    request_id: Uuid,
    token: String,
    password: String,
}

/// Creates the invited account; responds with its id.
#[utoipa::path(post, path = "/api/identity/invitations/accept", request_body = Accept, responses((status = 201, body = Uuid)))]
async fn accept(
    State(identity): State<Identity>,
    Json(input): Json<Accept>,
) -> Result<impl IntoResponse, IdentityError> {
    let id = identity
        .accept_invitation(&input.token, &input.password, input.request_id)
        .await?;
    Ok((StatusCode::CREATED, Json(id)))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
struct Revoke {
    request_id: Uuid,
    expected_revision: i64,
    id: Uuid,
}

#[utoipa::path(post, path = "/api/identity/invitations/revoke", operation_id = "revoke_invitation", request_body = Revoke, responses((status = 200, body = Receipt)))]
async fn revoke(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<Revoke>,
) -> Result<Json<Receipt>, IdentityError> {
    let actor = super::identity::actor(&identity, &headers).await?;
    let receipt = identity
        .revoke_invitation(&actor, input.id, input.expected_revision, input.request_id)
        .await?;
    Ok(Json(receipt))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Cursor {
    after: Option<Uuid>,
}

#[utoipa::path(get, path = "/api/identity/invitations", operation_id = "list_invitations", params(("after" = Option<Uuid>, Query)), responses((status = 200, body = Vec<crate::identity::InvitationSummary>)))]
async fn list(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Query(cursor): Query<Cursor>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(identity.invitations(&actor, cursor.after).await?),
    ))
}
