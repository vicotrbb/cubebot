use axum::{
    Json, Router,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::post,
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::identity::{Identity, IdentityError};

pub fn router(identity: Identity) -> Router {
    Router::new()
        .route("/api/identity/invitations", post(invite))
        .route("/api/identity/invitations/accept", post(accept))
        .route("/api/identity/invitations/revoke", post(revoke))
        .with_state(identity)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Invite {
    login: String,
}

#[derive(Serialize)]
struct Invitation {
    id: Uuid,
    token: String,
}

async fn invite(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<Invite>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&identity, &headers).await?;
    let invitation = identity.invite(&actor, &input.login).await?;
    Ok((
        StatusCode::CREATED,
        [("cache-control", "no-store")],
        Json(Invitation {
            id: invitation.id,
            token: invitation.token,
        }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Accept {
    token: String,
    password: String,
}

async fn accept(
    State(identity): State<Identity>,
    Json(input): Json<Accept>,
) -> Result<impl IntoResponse, IdentityError> {
    let id = identity
        .accept_invitation(&input.token, &input.password)
        .await?;
    Ok((StatusCode::CREATED, Json(id)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revoke {
    id: Uuid,
}

async fn revoke(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<Revoke>,
) -> Result<Json<bool>, IdentityError> {
    let actor = super::identity::actor(&identity, &headers).await?;
    identity.revoke_invitation(&actor, input.id).await?;
    Ok(Json(true))
}
