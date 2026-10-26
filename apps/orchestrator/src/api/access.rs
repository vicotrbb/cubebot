use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, post},
};
use serde::Deserialize;
use uuid::Uuid;

use crate::identity::{
    Identity, IdentityError, Receipt,
    access::{AccessExplanation, Capability, GrantRequest, RevokeGrant, Scope},
    teams::{CreateTeam, DeleteTeam, MembershipChange, TeamId},
};

pub fn router(identity: Identity) -> Router {
    Router::new()
        .route("/api/identity/teams", get(teams).post(create_team))
        .route("/api/identity/teams/membership", post(membership))
        .route("/api/identity/teams/delete", post(delete_team))
        .route("/api/identity/access/grants", post(grant))
        .route("/api/identity/access/revoke", post(revoke))
        .route("/api/identity/access/explain", post(explain))
        .route("/api/identity/access/events", get(events))
        .route("/api/identity/commands/{kind}/{request_id}", get(receipt))
        .with_state(identity)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TeamCursor {
    after: Option<TeamId>,
}

async fn teams(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Query(cursor): Query<TeamCursor>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&identity, &headers).await?;
    Ok(no_store(identity.teams(&actor, cursor.after).await?))
}

async fn create_team(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<CreateTeam>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&identity, &headers).await?;
    Ok((
        StatusCode::CREATED,
        no_store(identity.create_team(&actor, &input).await?),
    ))
}

async fn membership(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<MembershipChange>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&identity, &headers).await?;
    Ok(no_store(identity.change_membership(&actor, &input).await?))
}

async fn delete_team(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<DeleteTeam>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&identity, &headers).await?;
    Ok(no_store(identity.delete_team(&actor, &input).await?))
}

async fn grant(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<GrantRequest>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&identity, &headers).await?;
    Ok((
        StatusCode::CREATED,
        no_store(identity.grant(&actor, &input).await?),
    ))
}

async fn revoke(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<RevokeGrant>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&identity, &headers).await?;
    Ok(no_store(identity.revoke_grant(&actor, &input).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Explain {
    scope: Scope,
    capability: Capability,
}

async fn explain(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Json(input): Json<Explain>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&identity, &headers).await?;
    let explanation: AccessExplanation = identity
        .explain_access(&actor, input.scope, input.capability)
        .await?;
    Ok(no_store(explanation))
}

async fn receipt(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Path((kind, request_id)): Path<(String, Uuid)>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&identity, &headers).await?;
    let receipt: Receipt = identity.command_receipt(&actor, &kind, request_id).await?;
    Ok(no_store(receipt))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EventCursor {
    #[serde(default)]
    after: i64,
}

async fn events(
    State(identity): State<Identity>,
    headers: HeaderMap,
    Query(cursor): Query<EventCursor>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&identity, &headers).await?;
    Ok(no_store(
        identity.access_events(&actor, cursor.after).await?,
    ))
}

fn no_store(value: impl serde::Serialize) -> impl IntoResponse {
    ([("cache-control", "no-store")], Json(value))
}
