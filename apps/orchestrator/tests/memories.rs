use crate::support;
use cubebot_orchestrator::{
    agents::{AgentId, Agents, CreateAgent},
    identity::{Actor, Identity, IdentityError},
    knowledge::{self, CreateMemory, Knowledge, ReviseMemory},
    work::{SendMessage, Work},
};
use cubebot_protocol::runtime::MemoryChange;
use uuid::Uuid;

async fn agent(pool: &sqlx::PgPool, admin: &Actor, name: &str) -> AgentId {
    let created = Agents::new(pool.clone())
        .create(
            admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: name.into(),
                instructions: String::new(),
            },
        )
        .await
        .unwrap();
    AgentId(created.id)
}

async fn turn(pool: &sqlx::PgPool, admin: &Actor, agent: AgentId) -> Uuid {
    let input = SendMessage {
        request_id: Uuid::new_v4(),
        message: "The deadline moved".into(),
    };
    Work::new(pool.clone())
        .send(admin, agent.0, &input)
        .await
        .unwrap()
        .id
}

async fn commit(pool: &sqlx::PgPool, agent: AgentId, turn: Uuid, changes: &[MemoryChange]) {
    let mut tx = pool.begin().await.unwrap();
    knowledge::commit_turn(&mut tx, agent, turn, changes)
        .await
        .unwrap();
    tx.commit().await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn human_edits_win_over_stale_agent_writes_and_history_is_kept() {
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
    let writer = agent(&pool, &admin, "Writer").await;
    let other = agent(&pool, &admin, "Other").await;
    let knowledge = Knowledge::new(pool.clone());

    let deadline = knowledge
        .create(
            &admin,
            writer,
            &CreateMemory {
                content: "The deadline is November 15".into(),
            },
        )
        .await
        .unwrap();
    assert_eq!((deadline.revision, deadline.edited_by_human), (1, true));
    let source = turn(&pool, &admin, writer).await;
    let revise = MemoryChange::Revise {
        memory: deadline.id,
        expected_revision: 1,
        content: "The deadline is November 22".into(),
    };
    let remember = MemoryChange::Remember {
        content: "The project uses PostgreSQL 18".into(),
    };
    commit(&pool, writer, source, &[revise, remember.clone(), remember]).await;
    let found = knowledge
        .search(&admin, writer, Some("deadlines"))
        .await
        .unwrap();
    assert_eq!(found.len(), 1, "stemmed search finds the deadline only");
    assert_eq!(found[0].content, "The deadline is November 22");
    assert_eq!((found[0].revision, found[0].source_turn), (2, Some(source)));
    assert!(!found[0].edited_by_human);
    assert_eq!(
        knowledge.search(&admin, writer, None).await.unwrap().len(),
        2,
        "no duplicate"
    );

    // A person corrects the memory while the agent still holds revision 2.
    knowledge
        .revise(
            &admin,
            writer,
            deadline.id,
            &ReviseMemory {
                content: "The deadline is November 29".into(),
                expected_revision: 2,
            },
        )
        .await
        .unwrap();
    let stale = MemoryChange::Forget {
        memory: deadline.id,
        expected_revision: 2,
    };
    commit(&pool, writer, turn(&pool, &admin, writer).await, &[stale]).await;
    let kept = knowledge
        .search_for_agent(writer, "deadline")
        .await
        .unwrap();
    assert_eq!(
        (kept[0].revision, kept[0].content.as_str()),
        (3, "The deadline is November 29")
    );
    assert!(matches!(
        knowledge
            .revise(
                &admin,
                writer,
                deadline.id,
                &ReviseMemory {
                    content: "Stale".into(),
                    expected_revision: 2
                }
            )
            .await,
        Err(IdentityError::Conflict)
    ));

    // The agent cannot bring back verbatim what a person replaced, as a new memory or a revision.
    let rejected = "The deadline is November 22".to_owned();
    let replay = [
        MemoryChange::Remember {
            content: rejected.clone(),
        },
        MemoryChange::Revise {
            memory: deadline.id,
            expected_revision: 3,
            content: rejected,
        },
    ];
    commit(&pool, writer, turn(&pool, &admin, writer).await, &replay).await;
    let kept = knowledge
        .search_for_agent(writer, "deadline")
        .await
        .unwrap();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].revision, 3);

    // Another agent can neither find nor change this agent's memory.
    assert!(
        knowledge
            .search_for_agent(other, "deadline")
            .await
            .unwrap()
            .is_empty()
    );
    let foreign = MemoryChange::Forget {
        memory: deadline.id,
        expected_revision: 3,
    };
    commit(&pool, other, turn(&pool, &admin, other).await, &[foreign]).await;
    assert_eq!(
        knowledge
            .search_for_agent(writer, "deadline")
            .await
            .unwrap()
            .len(),
        1
    );

    // Any query word matches; stop words and tsquery syntax are only text.
    assert_eq!(
        knowledge
            .search_for_agent(writer, "postgresql launch")
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        knowledge
            .search_for_agent(writer, "the")
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        knowledge
            .search_for_agent(writer, r"it's C:\path & | !")
            .await
            .unwrap()
            .is_empty()
    );

    knowledge
        .delete(&admin, writer, deadline.id, 3)
        .await
        .unwrap();
    assert!(
        knowledge
            .search_for_agent(writer, "deadline")
            .await
            .unwrap()
            .is_empty()
    );
    let removed = MemoryChange::Remember {
        content: "The deadline is November 29".into(),
    };
    commit(&pool, writer, turn(&pool, &admin, writer).await, &[removed]).await;
    assert!(
        knowledge
            .search_for_agent(writer, "deadline")
            .await
            .unwrap()
            .is_empty(),
        "a person's removal is not undone verbatim"
    );
    let history: Vec<(i64, bool, bool)> = sqlx::query_as("SELECT revision,deleted,human_id IS NOT NULL FROM memory_revisions WHERE memory_id=$1 ORDER BY revision")
        .bind(deadline.id).fetch_all(&pool).await.unwrap();
    assert_eq!(
        history,
        [
            (1, false, true),
            (2, false, false),
            (3, false, true),
            (4, true, true)
        ]
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn memories_need_their_own_grants() {
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
    let writer = agent(&pool, &admin, "Writer").await;
    let knowledge = Knowledge::new(pool);
    assert!(matches!(
        knowledge.search(&member, writer, None).await,
        Err(IdentityError::NotFound)
    ));
    let input = CreateMemory {
        content: "Planted".into(),
    };
    assert!(matches!(
        knowledge.create(&member, writer, &input).await,
        Err(IdentityError::NotFound)
    ));
    let empty = CreateMemory {
        content: " ".into(),
    };
    assert!(matches!(
        knowledge.create(&admin, writer, &empty).await,
        Err(IdentityError::InvalidInput)
    ));
}
