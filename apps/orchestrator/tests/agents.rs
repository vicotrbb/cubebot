use crate::support;
use cubebot_orchestrator::{
    agents::{AgentId, Agents, CreateAgent},
    identity::{
        Identity, IdentityError,
        access::{Capability, GrantRequest, Recipient, Scope},
    },
};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn definitions_are_durable_scoped_and_creation_replays() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let admin_token = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&admin_token.token).await.unwrap();
    let member = identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let token = identity.login("member", "member password").await.unwrap();
    let actor = identity.authenticate(&token.token).await.unwrap();
    let agents = Agents::new(pool.clone());
    let input = CreateAgent {
        request_id: Uuid::new_v4(),
        name: "Researcher".into(),
        instructions: "Inspect evidence".into(),
    };
    assert!(matches!(
        agents.create(&actor, &input).await,
        Err(IdentityError::Forbidden)
    ));
    let receipt = agents.create(&admin, &input).await.unwrap();
    assert_eq!(agents.create(&admin, &input).await.unwrap(), receipt);
    assert!(agents.list(&actor, None).await.unwrap().is_empty());
    let grant = identity
        .grant(
            &admin,
            &GrantRequest {
                request_id: Uuid::new_v4(),
                recipient: Recipient::User { id: member },
                scope: Scope::Agent { id: receipt.id },
                capability: Capability::AgentManage,
            },
        )
        .await
        .unwrap();
    let definition = agents.get(&actor, AgentId(receipt.id)).await.unwrap();
    assert_eq!(definition.name, "Researcher");
    assert_eq!(definition.desired_state, "sleeping");
    sqlx::query!(
        "
        INSERT INTO agents (id, name, instructions, created_by)
        SELECT ('00000000-0000-0000-0000-' || lpad(to_hex(n), 12, '0'))::uuid, 'Hidden', 'Private instructions', $1
        FROM generate_series(1, 110) n
        ",
        admin.id,
    ).execute(&pool).await.unwrap();
    assert_eq!(agents.list(&actor, None).await.unwrap().len(), 1);
    assert!(
        agents
            .list(&actor, Some(AgentId(receipt.id)))
            .await
            .unwrap()
            .is_empty()
    );
    check_reconnect_and_revoke(pool, &identity, &admin, &actor, receipt.id, grant.id).await;
}

async fn check_reconnect_and_revoke(
    pool: sqlx::PgPool,
    identity: &Identity,
    admin: &cubebot_orchestrator::identity::Actor,
    actor: &cubebot_orchestrator::identity::Actor,
    id: Uuid,
    grant: Uuid,
) {
    let agents = Agents::new(pool);
    assert_eq!(agents.get(actor, AgentId(id)).await.unwrap().revision, 1);
    let status = agents.status(actor, AgentId(id)).await.unwrap();
    assert_eq!(status.desired_state, "sleeping");
    assert!(status.pod_attempt.is_none());
    identity
        .revoke_grant(
            admin,
            &cubebot_orchestrator::identity::access::RevokeGrant {
                request_id: Uuid::new_v4(),
                id: cubebot_orchestrator::identity::access::GrantId(grant),
                expected_revision: 1,
            },
        )
        .await
        .unwrap();
    assert!(agents.list(actor, None).await.unwrap().is_empty());
    assert!(matches!(
        agents.status(actor, AgentId(id)).await,
        Err(IdentityError::NotFound)
    ));
    assert!(matches!(
        agents.get(actor, AgentId(id)).await,
        Err(IdentityError::NotFound)
    ));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn creation_is_atomic_under_retries_and_rechecks_current_role() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let token = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let actor = identity.authenticate(&token.token).await.unwrap();
    let agents = Agents::new(pool.clone());
    let input = CreateAgent {
        request_id: Uuid::new_v4(),
        name: "Durable agent".into(),
        instructions: "Keep this configuration".into(),
    };
    let (first, second) =
        tokio::join!(agents.create(&actor, &input), agents.create(&actor, &input));
    let receipt = first.unwrap();
    assert_eq!(second.unwrap(), receipt);
    let changed = CreateAgent {
        name: "Changed".into(),
        ..input
    };
    assert!(matches!(
        agents.create(&actor, &changed).await,
        Err(IdentityError::Conflict)
    ));
    let count: i64 = sqlx::query_scalar!(
        r#"
        SELECT count(*) AS "count!"
        FROM agents
        "#
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    let count: i64 = sqlx::query_scalar!(
        r#"
        SELECT count(*) AS "count!"
        FROM agent_events
        "#
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
    sqlx::query!(
        "
        UPDATE human_accounts
        SET role = 'member'
        WHERE id = $1
        ",
        actor.id,
    )
    .execute(&pool)
    .await
    .unwrap();
    assert!(matches!(
        agents.create(&actor, &changed).await,
        Err(IdentityError::Forbidden)
    ));
    assert!(agents.list(&actor, None).await.unwrap().is_empty());
    let stored: String = sqlx::query_scalar!(
        "
        SELECT name
        FROM agents
        WHERE id = $1
        ",
        receipt.id,
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(stored, "Durable agent");
}
