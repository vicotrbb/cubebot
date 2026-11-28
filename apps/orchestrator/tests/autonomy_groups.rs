//! Group managers, group roles, single sign-on groups, personal connections and provider keys.
use crate::support;

use axum::Router;
use cubebot_orchestrator::{
    agents::{AgentId, Agents, CreateAgent},
    connections::{Connections, Credentials, ProviderAccess, SavePersonalProvider},
    identity::{
        Actor, Identity, IdentityError, Role,
        groups::{CreateGroup, GroupId, GroupRole, GroupSource, MembershipChange},
        members::ChangeRole,
    },
};
use cubebot_protocol::runtime::ProviderKind;
use serde_json::json;
use uuid::Uuid;

const PASSWORD: &str = "long enough password";

async fn sign_in(identity: &Identity, login: &str) -> Actor {
    let session = identity.login(login, PASSWORD).await.unwrap();
    identity.authenticate(&session.token).await.unwrap()
}

async fn setup() -> (Identity, sqlx::PgPool, Actor) {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity.bootstrap("owner", PASSWORD).await.unwrap();
    let owner = sign_in(&identity, "owner").await;
    (identity, pool, owner)
}

async fn add(identity: &Identity, owner: &Actor, login: &str, role: Role) -> Actor {
    identity
        .create_account_as(owner, login, PASSWORD, role, Uuid::new_v4())
        .await
        .unwrap();
    sign_in(identity, login).await
}

async fn group(
    identity: &Identity,
    actor: &Actor,
    name: &str,
    source: Option<(GroupSource, &str)>,
) -> GroupId {
    let receipt = identity
        .create_group(
            actor,
            &CreateGroup {
                request_id: Uuid::new_v4(),
                name: name.into(),
                source: source.map(|(source, _)| source),
                external_id: source.map(|(_, external)| external.into()),
            },
        )
        .await
        .unwrap();
    GroupId(receipt.id)
}

fn change(
    group: GroupId,
    account: Uuid,
    revision: i64,
    present: bool,
    manager: Option<bool>,
) -> MembershipChange {
    MembershipChange {
        request_id: Uuid::new_v4(),
        group,
        account_id: account,
        expected_revision: revision,
        present,
        manager,
    }
}

async fn role_of(pool: &sqlx::PgPool, id: Uuid) -> String {
    sqlx::query_scalar!("SELECT role FROM human_accounts WHERE id = $1", id)
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn group_managers_change_members_but_not_roles_or_managers() {
    let (identity, _pool, owner) = setup().await;
    let ada = add(&identity, &owner, "ada", Role::Member).await;
    let bob = add(&identity, &owner, "bob", Role::Member).await;
    let team = group(&identity, &owner, "Design", None).await;
    let made = identity
        .change_membership(&owner, &change(team, ada.id, 1, true, Some(true)))
        .await
        .unwrap();
    let added = identity
        .change_membership(&ada, &change(team, bob.id, made.revision, true, None))
        .await
        .unwrap();
    assert!(matches!(
        identity
            .change_membership(
                &ada,
                &change(team, bob.id, added.revision, true, Some(true))
            )
            .await,
        Err(IdentityError::Forbidden)
    ));
    assert!(matches!(
        identity
            .change_membership(&bob, &change(team, ada.id, added.revision, false, None))
            .await,
        Err(IdentityError::Forbidden)
    ));
    let listed = identity.groups(&ada, None).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].managers, vec![ada.id]);
    assert!(identity.groups(&bob, None).await.unwrap().is_empty());
    // A group carrying a role is the administrators' to fill.
    identity
        .set_group_role(
            &owner,
            &GroupRole {
                request_id: Uuid::new_v4(),
                group: team,
                role: Some(Role::Member),
                expected_revision: added.revision,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        identity
            .change_membership(&ada, &change(team, bob.id, added.revision + 1, false, None))
            .await,
        Err(IdentityError::Forbidden)
    ));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_group_role_follows_membership() {
    let (identity, pool, owner) = setup().await;
    let admin = add(&identity, &owner, "adm", Role::Administrator).await;
    let ada = add(&identity, &owner, "ada", Role::Member).await;
    let auditors = group(&identity, &owner, "Compliance", None).await;
    let joined = identity
        .change_membership(&owner, &change(auditors, ada.id, 1, true, None))
        .await
        .unwrap();
    let mapped = identity
        .set_group_role(
            &owner,
            &GroupRole {
                request_id: Uuid::new_v4(),
                group: auditors,
                role: Some(Role::Auditor),
                expected_revision: joined.revision,
            },
        )
        .await
        .unwrap();
    assert_eq!(role_of(&pool, ada.id).await, "auditor");
    let revision = sqlx::query_scalar!("SELECT revision FROM human_accounts WHERE id = $1", ada.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(matches!(
        identity
            .change_role(
                &owner,
                &ChangeRole {
                    request_id: Uuid::new_v4(),
                    id: ada.id,
                    role: Role::Member,
                    expected_revision: revision,
                },
            )
            .await,
        Err(IdentityError::Conflict)
    ));
    // Only the installation owner maps a group to administrators.
    assert!(matches!(
        identity
            .set_group_role(
                &admin,
                &GroupRole {
                    request_id: Uuid::new_v4(),
                    group: auditors,
                    role: Some(Role::Administrator),
                    expected_revision: mapped.revision,
                },
            )
            .await,
        Err(IdentityError::Forbidden)
    ));
    identity
        .change_membership(
            &owner,
            &change(auditors, ada.id, mapped.revision, false, None),
        )
        .await
        .unwrap();
    assert_eq!(role_of(&pool, ada.id).await, "member");
    // The installation owner keeps their role whatever group they join.
    let owners = identity.groups(&owner, None).await.unwrap()[0].revision;
    identity
        .change_membership(&owner, &change(auditors, owner.id, owners, true, None))
        .await
        .unwrap();
    assert_eq!(role_of(&pool, owner.id).await, "administrator");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn single_sign_on_groups_follow_the_claim() {
    let (identity, pool, owner) = setup().await;
    let ada = add(&identity, &owner, "ada", Role::Member).await;
    let engineering = group(
        &identity,
        &owner,
        "Engineering",
        Some((GroupSource::Oidc, "eng")),
    )
    .await;
    let local = group(&identity, &owner, "Local", None).await;
    assert!(matches!(
        identity
            .change_membership(&owner, &change(engineering, ada.id, 1, true, None))
            .await,
        Err(IdentityError::Conflict)
    ));
    let mut tx = pool.begin().await.unwrap();
    cubebot_orchestrator::identity::groups::sync_claimed(
        &mut tx,
        ada.id,
        &["eng".into(), "other".into()],
    )
    .await
    .unwrap();
    tx.commit().await.unwrap();
    let members = |id: GroupId| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar!(
                "SELECT account_id FROM human_team_members WHERE team_id = $1",
                id.0
            )
            .fetch_all(&pool)
            .await
            .unwrap()
        }
    };
    assert_eq!(members(engineering).await, vec![ada.id]);
    assert!(members(local).await.is_empty());
    let mut tx = pool.begin().await.unwrap();
    cubebot_orchestrator::identity::groups::sync_claimed(&mut tx, ada.id, &[])
        .await
        .unwrap();
    tx.commit().await.unwrap();
    assert!(members(engineering).await.is_empty());
}

fn agent(name: &str) -> CreateAgent {
    CreateAgent {
        request_id: Uuid::new_v4(),
        name: name.into(),
        instructions: "Help".into(),
        avatar: None,
        voice: None,
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn personal_connections_serve_only_their_owners_agents() {
    let (identity, pool, owner) = setup().await;
    let ada = add(&identity, &owner, "ada", Role::Member).await;
    let bob = add(&identity, &owner, "bob", Role::Member).await;
    let agents = Agents::new(pool.clone());
    let hers = AgentId(agents.create(&ada, &agent("Hers")).await.unwrap().id);
    let his = AgentId(agents.create(&bob, &agent("His")).await.unwrap().id);
    const FEED: &str = r#"<?xml version="1.0"?><rss version="2.0"><channel><title>News</title><item><title>One</title><link>https://example.com/1</link><guid>1</guid></item></channel></rss>"#;
    let router = Router::new().route(
        "/feed.xml",
        axum::routing::get(|| async { ([("content-type", "application/rss+xml")], FEED) }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let feed = format!("http://{}/feed.xml", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let connections = Connections::new(pool.clone(), Credentials::new([7; 32]));
    let rss = |slug: &str, access: serde_json::Value| {
        serde_json::from_value(json!({"connector": "rss", "slug": slug, "auth": "public",
            "config": {"feeds": [feed]}, "secrets": {}, "access": access}))
        .unwrap()
    };
    // A member's connection never opens to every agent, nor reaches someone else's.
    let everyone = connections
        .create_connection(ada.id, rss("everyone", json!({"mode": "all_agents"})))
        .await
        .unwrap();
    assert!(matches!(
        &everyone.access.0,
        ProviderAccess::ExplicitGrants { agents } if agents.is_empty()
    ));
    assert!(
        connections
            .create_connection(
                ada.id,
                rss(
                    "theirs",
                    json!({"mode": "explicit_grants", "agents": [his.0]})
                )
            )
            .await
            .is_err()
    );
    let saved = connections
        .create_connection(
            ada.id,
            rss(
                "mine",
                json!({"mode": "explicit_grants", "agents": [hers.0]}),
            ),
        )
        .await
        .unwrap();
    assert!(saved.personal && saved.owner == ada.id);
    let usable = |agent: AgentId| {
        let connections = connections.clone();
        async move {
            connections
                .agent_connections(agent)
                .await
                .unwrap()
                .into_iter()
                .any(|connection| connection.id == saved.id && connection.enabled)
        }
    };
    assert!(usable(hers).await);
    assert!(!usable(his).await);
    // Suspending the owner stops their agents using it.
    identity
        .suspend(&owner, ada.id, 1, Uuid::new_v4())
        .await
        .unwrap();
    assert!(!usable(hers).await);
    assert!(
        connections
            .personal_access(ada.id, &ProviderAccess::AllAgents)
            .await
            .is_err()
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn owners_bring_their_own_provider_keys() {
    let (identity, pool, owner) = setup().await;
    let ada = add(&identity, &owner, "ada", Role::Member).await;
    let agents = Agents::new(pool.clone());
    let hers = AgentId(agents.create(&ada, &agent("Hers")).await.unwrap().id);
    let theirs = AgentId(agents.create(&owner, &agent("Theirs")).await.unwrap().id);
    let connections = Connections::new(pool.clone(), Credentials::new([7; 32]));
    let key = SavePersonalProvider {
        api_key: "sk-ada".into(),
        context_window: None,
    };
    assert!(matches!(
        connections
            .save_personal_provider(ada.id, ProviderKind::Chatgpt, &key)
            .await,
        Err(IdentityError::InvalidInput)
    ));
    connections
        .save_personal_provider(ada.id, ProviderKind::Anthropic, &key)
        .await
        .unwrap();
    let listed = connections.personal_providers(ada.id).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert!(!serde_json::to_string(&listed).unwrap().contains("sk-ada"));
    let model = connections.agent_model(hers).await.unwrap();
    assert_eq!(model.providers, vec![ProviderKind::Anthropic]);
    assert!(
        connections
            .agent_model(theirs)
            .await
            .unwrap()
            .providers
            .is_empty()
    );
    let stored: Vec<u8> = sqlx::query_scalar!("SELECT credential FROM personal_providers")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&stored).contains("sk-ada"));
    connections
        .forget_personal_provider(ada.id, ProviderKind::Anthropic)
        .await
        .unwrap();
    assert!(
        connections
            .agent_model(hers)
            .await
            .unwrap()
            .providers
            .is_empty()
    );
}
