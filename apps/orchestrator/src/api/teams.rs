use crate::{
    identity::{Actor, Identity, IdentityError, audit::Event},
    work::{CreateTeam, Team, UpdateTeam, Work},
};
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, put},
};
use serde::Deserialize;
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    work: Work,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(list_teams, create_team, update_team, delete_team))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/teams", get(list_teams).post(create_team))
        .route("/api/teams/{team}", put(update_team).delete(delete_team))
        .with_state(Api {
            identity,
            work: Work::new(pool),
        })
}

/// Administrators see every team; others the teams whose room they can read.
#[utoipa::path(get, path = "/api/teams", responses((status = 200, body = Vec<Team>)))]
async fn list_teams(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.work.teams(&actor).await?),
    ))
}

/// Administrators only; also creates the team's room. `request_id` makes a retry return the same team.
#[utoipa::path(post, path = "/api/teams", request_body = CreateTeam, responses((status = 201, body = Team)))]
async fn create_team(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<CreateTeam>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let team = api.work.create_team(&actor, &input).await?;
    audit(&api, &actor, "agent_team.create", &team).await;
    Ok((
        StatusCode::CREATED,
        [("cache-control", "no-store")],
        Json(team),
    ))
}

/// Replaces the team's text, place and roster; administrators only.
#[utoipa::path(put, path = "/api/teams/{team}", params(("team" = Uuid, Path)), request_body = UpdateTeam, responses((status = 200, body = Team)))]
async fn update_team(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(team): Path<Uuid>,
    Json(input): Json<UpdateTeam>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let team = api.work.update_team(&actor, team, &input).await?;
    audit(&api, &actor, "agent_team.update", &team).await;
    Ok(([("cache-control", "no-store")], Json(team)))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Revision {
    expected_revision: i64,
}

/// Deletes a team with no team under it and archives its room; administrators only.
#[utoipa::path(delete, path = "/api/teams/{team}", params(("team" = Uuid, Path), ("expected_revision" = i64, Query)), responses((status = 200, body = Team)))]
async fn delete_team(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(team): Path<Uuid>,
    Query(revision): Query<Revision>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    let team = api
        .work
        .delete_team(&actor, team, revision.expected_revision)
        .await?;
    audit(&api, &actor, "agent_team.delete", &team).await;
    Ok(([("cache-control", "no-store")], Json(team)))
}

async fn audit(api: &Api, actor: &Actor, kind: &'static str, team: &Team) {
    let members: Vec<_> = team.members.iter().map(|member| &member.name).collect();
    let detail = serde_json::json!({ "name": team.name, "members": members });
    let event = Event::by(actor.id, kind).on(team.id).detail(detail);
    api.identity.audit(event).await;
}
