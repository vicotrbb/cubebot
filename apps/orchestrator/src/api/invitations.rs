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

pub fn router(identity: Identity) -> Router {
    Router::new()
        .route("/api/identity/invitations", get(list).post(invite))
        .route("/api/identity/invitations/accept", post(accept))
        .route("/api/identity/invitations/revoke", post(revoke))
        .with_state(identity)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invite {
    request_id: Uuid,
    login: String,
}

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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Accept {
    request_id: Uuid,
    token: String,
    password: String,
}

async fn accept(
    State(identity): State<Identity>,
    Json(input): Json<Accept>,
) -> Result<impl IntoResponse, IdentityError> {
    let id = identity
        .accept_invitation(&input.token, &input.password, input.request_id)
        .await?;
    Ok((StatusCode::CREATED, Json(id)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revoke {
    request_id: Uuid,
    expected_revision: i64,
    id: Uuid,
}

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
