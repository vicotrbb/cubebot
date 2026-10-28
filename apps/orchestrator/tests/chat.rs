use crate::support;
use cubebot_orchestrator::{
    agents::{AgentId, Agents, CreateAgent},
    identity::{Identity, IdentityError},
    work::{SendMessage, Work},
};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn accepted_messages_survive_reconnect_and_concurrent_retries_are_unique() {
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
    let agent = Agents::new(pool.clone())
        .create(
            &admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: "Writer".into(),
                instructions: "Be brief".into(),
            },
        )
        .await
        .unwrap();
    let work = Work::new(pool.clone());
    let conversation = work.conversation(&admin, AgentId(agent.id)).await.unwrap();
    let input = SendMessage {
        request_id: Uuid::new_v4(),
        message: "Remember this".into(),
    };
    let (first, second) = tokio::join!(
        work.send(&admin, conversation.id, &input),
        work.send(&admin, conversation.id, &input)
    );
    assert_eq!(first.unwrap().id, second.unwrap().id);
    let restarted = Work::new(pool.clone());
    let history = restarted.history(&admin, conversation.id, 0).await.unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].message, "Remember this");
    assert_eq!(history[0].status, "queued");
    let changed = SendMessage {
        message: "Changed".into(),
        ..input
    };
    assert!(matches!(
        work.send(&admin, conversation.id, &changed).await,
        Err(IdentityError::Conflict)
    ));
    assert_eq!(
        Agents::new(pool)
            .get(&admin, AgentId(agent.id))
            .await
            .unwrap()
            .desired_state,
        "running"
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn ordinary_members_cannot_read_or_send_to_another_conversation() {
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
    identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let session = identity.login("member", "member password").await.unwrap();
    let member = identity.authenticate(&session.token).await.unwrap();
    let agent = Agents::new(pool.clone())
        .create(
            &admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: "Writer".into(),
                instructions: "Be brief".into(),
            },
        )
        .await
        .unwrap();
    let work = Work::new(pool);
    let conversation = work.conversation(&admin, AgentId(agent.id)).await.unwrap();
    assert!(matches!(
        work.history(&member, conversation.id, 0).await,
        Err(IdentityError::NotFound)
    ));
    let message = SendMessage {
        request_id: Uuid::new_v4(),
        message: "Intrude".into(),
    };
    assert!(matches!(
        work.send(&member, conversation.id, &message).await,
        Err(IdentityError::NotFound)
    ));
    assert!(matches!(
        work.conversation(&member, AgentId(agent.id)).await,
        Err(IdentityError::NotFound)
    ));
    assert_explicit_shared_access(&identity, &work, &admin, &member, AgentId(agent.id)).await;
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn every_agent_has_exactly_one_conversation_even_before_it_is_opened() {
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
    let agent = Agents::new(pool.clone())
        .create(
            &admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: "Single stream".into(),
                instructions: "Continue this context".into(),
            },
        )
        .await
        .unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM conversations WHERE agent_id=$1")
        .bind(agent.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let work = Work::new(pool.clone());
    let (first, second) = tokio::join!(
        work.conversation(&admin, AgentId(agent.id)),
        work.conversation(&admin, AgentId(agent.id))
    );
    assert_eq!(first.unwrap().id, agent.id);
    assert_eq!(second.unwrap().id, agent.id);
    assert!(
        sqlx::query("INSERT INTO conversations (id,agent_id,created_by) VALUES ($1,$2,$3)")
            .bind(Uuid::new_v4())
            .bind(agent.id)
            .bind(admin.id)
            .execute(&pool)
            .await
            .is_err()
    );
    assert_eq!(
        Work::new(pool)
            .conversation(&admin, AgentId(agent.id))
            .await
            .unwrap()
            .id,
        agent.id
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn legacy_histories_merge_without_broadening_private_grants() {
    let pool = support::create_database(None).await;
    let migrations = sqlx::migrate!("./migrations");
    for migration in migrations.iter().filter(|migration| migration.version < 23) {
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    let Legacy {
        admin,
        member,
        agent,
        first,
        second,
    } = seed_legacy(&pool).await;
    for migration in migrations
        .iter()
        .filter(|migration| migration.version >= 23)
    {
        sqlx::raw_sql(&migration.sql).execute(&pool).await.unwrap();
    }
    let actor = cubebot_orchestrator::identity::Actor {
        id: admin,
        administrator: true,
    };
    let history = Work::new(pool.clone())
        .history(&actor, agent, 0)
        .await
        .unwrap();
    assert_eq!(
        history
            .iter()
            .map(|turn| turn.message.as_str())
            .collect::<Vec<_>>(),
        vec!["First history", "Second history", "First again"]
    );
    let member_actor = cubebot_orchestrator::identity::Actor {
        id: member,
        administrator: false,
    };
    assert!(matches!(
        Work::new(pool.clone())
            .history(&member_actor, agent, 0)
            .await,
        Err(IdentityError::NotFound)
    ));
    let original_ids: Vec<Uuid> =
        sqlx::query_scalar("SELECT id FROM conversation_origins ORDER BY created_at")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(original_ids, vec![first, second]);
    assert_legacy_provenance(&pool, first, second).await;
    let request: Uuid = sqlx::query_scalar("SELECT request_id FROM chat_turns WHERE id=$1")
        .bind(history[0].id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        Work::new(pool)
            .send(
                &actor,
                first,
                &SendMessage {
                    request_id: request,
                    message: "First history".into()
                }
            )
            .await
            .unwrap()
            .id,
        history[0].id
    );
}

async fn legacy_conversation(pool: &sqlx::PgPool, agent: Uuid, actor: Uuid, message: &str) -> Uuid {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO conversations (agent_id,created_by,request_id) VALUES ($1,$2,$3) RETURNING id",
    )
    .bind(agent)
    .bind(actor)
    .bind(Uuid::new_v4())
    .fetch_one(pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO chat_turns (conversation_id,actor_id,request_id,message) VALUES ($1,$2,$3,$4)",
    )
    .bind(id)
    .bind(actor)
    .bind(Uuid::new_v4())
    .bind(message)
    .execute(pool)
    .await
    .unwrap();
    id
}

async fn assert_legacy_provenance(pool: &sqlx::PgPool, first: Uuid, second: Uuid) {
    let origins: Vec<Uuid> =
        sqlx::query_scalar("SELECT origin_conversation_id FROM chat_turns ORDER BY sequence")
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(origins, vec![first, second, first]);
    let origin: Uuid = sqlx::query_scalar(
        "SELECT origin_conversation_id FROM access_grants WHERE revoked_at IS NOT NULL",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(origin, first);
}

async fn assert_explicit_shared_access(
    identity: &Identity,
    work: &Work,
    admin: &cubebot_orchestrator::identity::Actor,
    member: &cubebot_orchestrator::identity::Actor,
    agent: AgentId,
) {
    use cubebot_orchestrator::identity::access::{Capability, GrantRequest, Recipient, Scope};
    identity
        .grant(
            admin,
            &GrantRequest {
                request_id: Uuid::new_v4(),
                recipient: Recipient::User { id: member.id },
                scope: Scope::Agent { id: agent.0 },
                capability: Capability::AgentManage,
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        work.conversation(member, agent).await,
        Err(IdentityError::NotFound)
    ));
    for capability in [Capability::ConversationRead, Capability::ConversationSend] {
        identity
            .grant(
                admin,
                &GrantRequest {
                    request_id: Uuid::new_v4(),
                    recipient: Recipient::User { id: member.id },
                    scope: Scope::Conversation { id: agent.0 },
                    capability,
                },
            )
            .await
            .unwrap();
    }
    assert_eq!(work.conversation(member, agent).await.unwrap().id, agent.0);
    work.send(
        member,
        agent.0,
        &SendMessage {
            request_id: Uuid::new_v4(),
            message: "Shared continuous context".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        work.history(admin, agent.0, 0).await.unwrap()[0].message,
        "Shared continuous context"
    );
}

struct Legacy {
    admin: Uuid,
    member: Uuid,
    agent: Uuid,
    first: Uuid,
    second: Uuid,
}
async fn seed_legacy(pool: &sqlx::PgPool) -> Legacy {
    let admin: Uuid = sqlx::query_scalar(
        "INSERT INTO human_accounts (login,role) VALUES ('admin','administrator') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let member: Uuid = sqlx::query_scalar(
        "INSERT INTO human_accounts (login,role) VALUES ('member','member') RETURNING id",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    let agent:Uuid=sqlx::query_scalar("INSERT INTO agents (name,instructions,created_by) VALUES ('Legacy','Retain history',$1) RETURNING id").bind(admin).fetch_one(pool).await.unwrap();
    let first = legacy_conversation(pool, agent, admin, "First history").await;
    let second = legacy_conversation(pool, agent, member, "Second history").await;
    sqlx::query("INSERT INTO access_grants (account_id,scope_kind,scope_id,capability) VALUES ($1,'conversation',$2,'conversation.read')").bind(member).bind(first).execute(pool).await.unwrap();
    sqlx::query("INSERT INTO chat_turns (conversation_id,actor_id,request_id,message) VALUES ($1,$2,$3,'First again')").bind(first).bind(admin).bind(Uuid::new_v4()).execute(pool).await.unwrap();
    Legacy {
        admin,
        member,
        agent,
        first,
        second,
    }
}
