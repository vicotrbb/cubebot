use crate::support;
use cubebot_orchestrator::{
    agents::{
        AgentId, Agents, CreateAgent, IdlePolicy, LifecycleAction, LifecycleCommand, UpdateAgent,
    },
    identity::{Identity, IdentityError},
};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn revisions_and_lifecycle_intent_are_durable_and_retirement_is_terminal() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let actor = identity.authenticate(&session.token).await.unwrap();
    let agents = Agents::new(pool.clone());
    let created = agents
        .create(
            &actor,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: "Researcher".into(),
                instructions: "Original".into(),
            },
        )
        .await
        .unwrap();
    let id = AgentId(created.id);
    let update = UpdateAgent {
        request_id: Uuid::new_v4(),
        expected_revision: 1,
        name: "Researcher v2".into(),
        instructions: "Corrected evidence".into(),
        idle_policy: IdlePolicy::KeepRunning,
        idle_delay_seconds: 900,
        dreaming_model: Some("dream-model".into()),
        task_lanes: Some(3),
    };
    let receipt = agents.update(&actor, id, &update).await.unwrap();
    assert_eq!(receipt.revision, 2);
    assert_eq!(agents.update(&actor, id, &update).await.unwrap(), receipt);
    let definition = agents.get(&actor, id).await.unwrap();
    assert_eq!(definition.instructions_revision, 2);
    assert_eq!(
        (
            definition.idle_delay_seconds,
            definition.dreaming_model.as_deref(),
            definition.task_lanes
        ),
        (900, Some("dream-model"), 3)
    );
    let invalid = UpdateAgent {
        request_id: Uuid::new_v4(),
        expected_revision: 2,
        idle_delay_seconds: 59,
        ..update
    };
    assert!(agents.update(&actor, id, &invalid).await.is_err());
    let too_many_lanes = UpdateAgent {
        request_id: Uuid::new_v4(),
        expected_revision: 2,
        idle_delay_seconds: 900,
        task_lanes: Some(5),
        ..invalid
    };
    assert!(agents.update(&actor, id, &too_many_lanes).await.is_err());
    let history = agents.revisions(&actor, id, 0).await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(
        (history[1].idle_delay_seconds, history[1].task_lanes),
        (900, 3)
    );
    assert_eq!(history[0].instructions, "Original");
    let (wake, receipt) = check_wake(&agents, &actor, id).await;
    check_restart_and_retire(&agents, &actor, id, wake, receipt).await;
    let retained: String = sqlx::query_scalar("SELECT instructions FROM agents WHERE id = $1")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(retained, "Corrected evidence");
}

async fn check_wake(
    agents: &Agents,
    actor: &cubebot_orchestrator::identity::Actor,
    id: AgentId,
) -> (LifecycleCommand, cubebot_orchestrator::identity::Receipt) {
    let wake = LifecycleCommand {
        request_id: Uuid::new_v4(),
        expected_revision: 2,
        action: LifecycleAction::Wake,
    };
    let receipt = agents.lifecycle(actor, id, &wake).await.unwrap();
    assert_eq!(agents.lifecycle(actor, id, &wake).await.unwrap(), receipt);
    assert_eq!(agents.get(actor, id).await.unwrap().restart_generation, 1);
    let stale = LifecycleCommand {
        request_id: Uuid::new_v4(),
        ..wake
    };
    assert!(matches!(
        agents.lifecycle(actor, id, &stale).await,
        Err(IdentityError::Conflict)
    ));
    let repeated_wake = LifecycleCommand {
        request_id: Uuid::new_v4(),
        expected_revision: 3,
        action: LifecycleAction::Wake,
    };
    agents.lifecycle(actor, id, &repeated_wake).await.unwrap();
    assert_eq!(agents.get(actor, id).await.unwrap().restart_generation, 1);
    (wake, receipt)
}

async fn check_restart_and_retire(
    agents: &Agents,
    actor: &cubebot_orchestrator::identity::Actor,
    id: AgentId,
    wake: LifecycleCommand,
    receipt: cubebot_orchestrator::identity::Receipt,
) {
    let restart = LifecycleCommand {
        request_id: Uuid::new_v4(),
        expected_revision: 4,
        action: LifecycleAction::Restart,
    };
    let restarted = agents.lifecycle(actor, id, &restart).await.unwrap();
    assert_eq!(
        agents.lifecycle(actor, id, &restart).await.unwrap(),
        restarted
    );
    assert_eq!(agents.get(actor, id).await.unwrap().restart_generation, 2);
    let retire = LifecycleCommand {
        request_id: Uuid::new_v4(),
        expected_revision: 5,
        action: LifecycleAction::Retire,
    };
    agents.lifecycle(actor, id, &retire).await.unwrap();
    assert_eq!(agents.lifecycle(actor, id, &wake).await.unwrap(), receipt);
    assert_eq!(
        agents.get(actor, id).await.unwrap().desired_state,
        "retired"
    );
    let wake = LifecycleCommand {
        request_id: Uuid::new_v4(),
        expected_revision: 6,
        action: LifecycleAction::Wake,
    };
    assert!(matches!(
        agents.lifecycle(actor, id, &wake).await,
        Err(IdentityError::Conflict)
    ));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn competing_managers_commit_one_revision_without_changing_instruction_identity() {
    use cubebot_orchestrator::identity::{
        Actor,
        access::{Capability, GrantRequest, Recipient, Scope},
    };
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    let manager_id = sqlx::query_scalar(
        "INSERT INTO human_accounts (login, role) VALUES ('manager', 'member') RETURNING id",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let manager = Actor {
        id: manager_id,
        administrator: false,
    };
    let agents = Agents::new(pool.clone());
    let created = agents
        .create(
            &admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: "Original".into(),
                instructions: "Keep this text".into(),
            },
        )
        .await
        .unwrap();
    let id = AgentId(created.id);
    let grant = identity
        .grant(
            &admin,
            &GrantRequest {
                request_id: Uuid::new_v4(),
                recipient: Recipient::User { id: manager_id },
                scope: Scope::Agent { id: id.0 },
                capability: Capability::AgentManage,
            },
        )
        .await
        .unwrap();
    let first = UpdateAgent {
        request_id: Uuid::new_v4(),
        expected_revision: 1,
        name: "First".into(),
        instructions: "Keep this text".into(),
        idle_policy: IdlePolicy::KeepRunning,
        idle_delay_seconds: 600,
        dreaming_model: None,
        task_lanes: None,
    };
    let second = UpdateAgent {
        request_id: Uuid::new_v4(),
        name: "Second".into(),
        expected_revision: 1,
        instructions: first.instructions.clone(),
        idle_policy: first.idle_policy,
        idle_delay_seconds: 600,
        dreaming_model: None,
        task_lanes: None,
    };
    let (first, second) = tokio::join!(
        agents.update(&admin, id, &first),
        agents.update(&manager, id, &second)
    );
    assert_eq!(usize::from(first.is_ok()) + usize::from(second.is_ok()), 1);
    assert!(
        matches!(first, Err(IdentityError::Conflict))
            || matches!(second, Err(IdentityError::Conflict))
    );
    let current = agents.get(&admin, id).await.unwrap();
    assert_eq!(current.revision, 2);
    assert_eq!(current.instructions_revision, 1);
    let history = agents.revisions(&admin, id, 0).await.unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[1].instructions, "Keep this text");
    let receipts: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_commands")
        .fetch_one(&pool)
        .await
        .unwrap();
    let events: i64 = sqlx::query_scalar("SELECT count(*) FROM agent_events")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((receipts, events), (2, 2));
    check_revocation_while_waiting(&pool, &identity, &admin, manager, id, grant.id).await;
}

async fn check_revocation_while_waiting(
    pool: &sqlx::PgPool,
    identity: &Identity,
    admin: &cubebot_orchestrator::identity::Actor,
    manager: cubebot_orchestrator::identity::Actor,
    id: AgentId,
    grant: Uuid,
) {
    use cubebot_orchestrator::identity::access::{GrantId, RevokeGrant};
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM agents WHERE id = $1 FOR UPDATE")
        .bind(id)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let agents = Agents::new(pool.clone());
    let pending = tokio::spawn(async move {
        agents
            .lifecycle(
                &manager,
                id,
                &LifecycleCommand {
                    request_id: Uuid::new_v4(),
                    expected_revision: 2,
                    action: LifecycleAction::Wake,
                },
            )
            .await
    });
    wait_for_agent_lock(pool).await;
    identity
        .revoke_grant(
            admin,
            &RevokeGrant {
                request_id: Uuid::new_v4(),
                id: GrantId(grant),
                expected_revision: 1,
            },
        )
        .await
        .unwrap();
    blocker.rollback().await.unwrap();
    assert!(matches!(
        pending.await.unwrap(),
        Err(IdentityError::NotFound)
    ));
}

async fn wait_for_agent_lock(pool: &sqlx::PgPool) {
    use std::time::Duration;
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let waiting:bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname = current_database() AND wait_event_type = 'Lock' AND query LIKE '%FOR UPDATE OF g%')").fetch_one(pool).await.unwrap();
            if waiting { break; }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }).await.unwrap();
}
