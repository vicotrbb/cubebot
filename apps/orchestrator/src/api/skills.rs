use crate::{
    agents::AgentId,
    identity::{Identity, IdentityError},
    knowledge::{InstallSkill, Knowledge, PublishSkill, ReviseSkill},
};
use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, Path, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
    routing::{get, put},
};
use uuid::Uuid;

#[derive(Clone)]
struct Api {
    identity: Identity,
    knowledge: Knowledge,
}

#[derive(utoipa::OpenApi)]
#[openapi(paths(
    list_skills,
    publish_skill,
    revise_skill,
    skill_revision,
    installed_skills,
    install_skill,
    uninstall_skill
))]
pub(super) struct Doc;

pub(super) fn router(pool: sqlx::PgPool, identity: Identity) -> Router {
    Router::new()
        .route("/api/skills", get(list_skills).post(publish_skill))
        .route("/api/skills/{skill}", put(revise_skill))
        .route(
            "/api/skills/{skill}/revisions/{revision}",
            get(skill_revision),
        )
        .route("/api/agents/{agent}/skills", get(installed_skills))
        .route(
            "/api/agents/{agent}/skills/{skill}",
            put(install_skill).delete(uninstall_skill),
        )
        // A package of at most 64 KiB, JSON-escaped.
        .layer(DefaultBodyLimit::max(160 * 1024))
        .with_state(Api {
            identity,
            knowledge: Knowledge::new(pool),
        })
}

/// The installation's shared library, each skill at its current revision.
#[utoipa::path(get, path = "/api/skills", responses((status = 200, body = Vec<crate::knowledge::Skill>)))]
async fn list_skills(
    State(api): State<Api>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.knowledge.skills(&actor).await?),
    ))
}

/// 409 when the name in `SKILL.md` is taken.
#[utoipa::path(post, path = "/api/skills", request_body = PublishSkill, responses((status = 201, body = crate::knowledge::Skill)))]
async fn publish_skill(
    State(api): State<Api>,
    headers: HeaderMap,
    Json(input): Json<PublishSkill>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        StatusCode::CREATED,
        [("cache-control", "no-store")],
        Json(api.knowledge.publish_skill(&actor, &input).await?),
    ))
}

/// Adds a revision; 409 when `expected_revision` is no longer current.
#[utoipa::path(put, path = "/api/skills/{skill}", params(("skill" = Uuid, Path)), request_body = ReviseSkill, responses((status = 200, body = crate::knowledge::Skill)))]
async fn revise_skill(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(skill): Path<Uuid>,
    Json(input): Json<ReviseSkill>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.knowledge.revise_skill(&actor, skill, &input).await?),
    ))
}

#[utoipa::path(get, path = "/api/skills/{skill}/revisions/{revision}", params(("skill" = Uuid, Path), ("revision" = i64, Path)), responses((status = 200, body = crate::knowledge::SkillRevision)))]
async fn skill_revision(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((skill, revision)): Path<(Uuid, i64)>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(
            api.knowledge
                .skill_revision(&actor, skill, revision)
                .await?,
        ),
    ))
}

#[utoipa::path(get, path = "/api/agents/{agent}/skills", params(("agent" = Uuid, Path)), responses((status = 200, body = Vec<crate::knowledge::InstalledSkill>)))]
async fn installed_skills(
    State(api): State<Api>,
    headers: HeaderMap,
    Path(agent): Path<AgentId>,
) -> Result<impl IntoResponse, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    Ok((
        [("cache-control", "no-store")],
        Json(api.knowledge.installed_skills(&actor, agent).await?),
    ))
}

/// Installs a revision on the agent, or moves its installation to it.
#[utoipa::path(put, path = "/api/agents/{agent}/skills/{skill}", params(("agent" = Uuid, Path), ("skill" = Uuid, Path)), request_body = InstallSkill, responses((status = 204)))]
async fn install_skill(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, skill)): Path<(AgentId, Uuid)>,
    Json(input): Json<InstallSkill>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.knowledge
        .install_skill(&actor, agent, skill, input.revision)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// The skill stays in the library.
#[utoipa::path(delete, path = "/api/agents/{agent}/skills/{skill}", params(("agent" = Uuid, Path), ("skill" = Uuid, Path)), responses((status = 204)))]
async fn uninstall_skill(
    State(api): State<Api>,
    headers: HeaderMap,
    Path((agent, skill)): Path<(AgentId, Uuid)>,
) -> Result<StatusCode, IdentityError> {
    let actor = super::identity::actor(&api.identity, &headers).await?;
    api.knowledge.uninstall_skill(&actor, agent, skill).await?;
    Ok(StatusCode::NO_CONTENT)
}
