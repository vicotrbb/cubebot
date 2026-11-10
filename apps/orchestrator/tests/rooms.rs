use crate::support;
use cubebot_orchestrator::{
    agents::{Agents, CreateAgent},
    connections::{Connections, Credentials},
    identity::{
        Actor, Identity, IdentityError,
        access::{Capability, GrantRequest, Recipient, Scope},
    },
    work::{CreateRoom, RoomMembers, SendMessage, Work},
};
use uuid::Uuid;

pub(crate) async fn agent(pool: &sqlx::PgPool, admin: &Actor, name: &str) -> Uuid {
    Agents::new(pool.clone())
        .create(
            admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: name.into(),
                instructions: format!("You are {name}."),
                avatar: None,
                voice: None,
            },
        )
        .await
        .unwrap()
        .id
}

fn message(text: &str) -> SendMessage {
    SendMessage {
        request_id: Uuid::new_v4(),
        message: text.into(),
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn mentions_deliver_once_per_member_and_other_messages_wait_for_the_coordinator() {
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
    let ada = agent(&pool, &admin, "Ada").await;
    let bob = agent(&pool, &admin, "Bob").await;
    let outsider = agent(&pool, &admin, "Cy").await;
    let work = Work::new(pool.clone());
    let create = CreateRoom {
        request_id: Uuid::new_v4(),
        name: "Launch".into(),
        agents: vec![ada, bob, ada],
    };
    let room = work.create_room(&admin, &create).await.unwrap();
    assert_eq!(work.create_room(&admin, &create).await.unwrap().id, room.id);
    assert_eq!(room.members.len(), 2);
    let invalid = CreateRoom {
        request_id: Uuid::new_v4(),
        name: "Invalid".into(),
        agents: vec![Uuid::new_v4()],
    };
    assert!(matches!(
        work.create_room(&admin, &invalid).await,
        Err(IdentityError::InvalidInput)
    ));
    let empty = CreateRoom {
        request_id: Uuid::new_v4(),
        name: "Empty".into(),
        agents: vec![],
    };
    let empty = work.create_room(&admin, &empty).await.unwrap();
    assert!(empty.members.is_empty());
    let unread = work
        .post_room_message(&admin, empty.id, &message("Anyone here?"))
        .await
        .unwrap();
    assert_eq!(unread.routing, "no_action");
    assert_eq!(unread.note.as_deref(), Some("no_agents"));
    assert!(unread.deliveries.is_empty());

    let mentioned = message("@Ada and @bob, then @ada again; cc @Cy");
    let (first, second) = tokio::join!(
        work.post_room_message(&admin, room.id, &mentioned),
        work.post_room_message(&admin, room.id, &mentioned)
    );
    let (first, second) = (first.unwrap(), second.unwrap());
    assert_eq!(first.id, second.id);
    assert_eq!(first.routing, "routed");
    let mut recipients: Vec<Uuid> = first.deliveries.iter().map(|d| d.agent_id).collect();
    recipients.sort();
    let mut expected = vec![ada, bob];
    expected.sort();
    assert_eq!(recipients, expected);
    assert!(work.history(&admin, ada, 0).await.unwrap().is_empty());
    let delivered: String = sqlx::query_scalar(
        "SELECT message FROM chat_turns WHERE conversation_id=$1 AND room_message_id IS NOT NULL",
    )
    .bind(ada)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(delivered.starts_with("In room Launch, admin wrote:"));
    let unread = cubebot_orchestrator::agents::Agents::new(pool.clone())
        .list(&admin, None)
        .await
        .unwrap();
    assert!(
        unread.iter().all(|agent| agent.last_sequence == 0),
        "a room message must not mark the agent's own conversation unread"
    );
    assert!(work.history(&admin, outsider, 0).await.unwrap().is_empty());
    let changed = SendMessage {
        message: "@Ada only".into(),
        ..mentioned
    };
    assert!(matches!(
        work.post_room_message(&admin, room.id, &changed).await,
        Err(IdentityError::Conflict)
    ));

    let open = work
        .post_room_message(&admin, room.id, &message("Can someone check the numbers?"))
        .await
        .unwrap();
    assert_eq!(
        (open.routing.as_str(), open.deliveries.len()),
        ("routing", 0)
    );
    let connections = Connections::new(pool.clone(), Credentials::new([7; 32]));
    let mut left_routing = Vec::new();
    for _ in 0..3 {
        left_routing.push(
            cubebot_orchestrator::work::route(&pool, &connections)
                .await
                .unwrap(),
        );
    }
    assert_eq!(left_routing, [false, false, true]);
    assert!(
        !cubebot_orchestrator::work::route(&pool, &connections)
            .await
            .unwrap()
    );
    let messages = work.room_messages(&admin, room.id, 0).await.unwrap();
    let settled = messages.iter().find(|m| m.id == open.id).unwrap();
    assert_eq!(settled.routing, "routed");
    assert_eq!(settled.note, None);
    assert_eq!(settled.deliveries.len(), 1);
    assert_eq!(settled.deliveries[0].agent, "Ada");
    assert!(work.history(&admin, ada, 0).await.unwrap().is_empty());
    assert!(work.history(&admin, bob, 0).await.unwrap().is_empty());

    identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let session = identity.login("member", "member password").await.unwrap();
    let member = identity.authenticate(&session.token).await.unwrap();
    assert!(work.rooms(&member).await.unwrap().is_empty());
    assert!(matches!(
        work.post_room_message(&member, room.id, &message("@Ada hi"))
            .await,
        Err(IdentityError::NotFound)
    ));
    for capability in [Capability::ConversationRead, Capability::ConversationSend] {
        identity
            .grant(
                &admin,
                &GrantRequest {
                    request_id: Uuid::new_v4(),
                    recipient: Recipient::User { id: member.id },
                    scope: Scope::Room { id: room.id },
                    capability,
                },
            )
            .await
            .unwrap();
    }
    let visible = work.rooms(&member).await.unwrap();
    assert_eq!(visible[0].id, room.id);
    assert_eq!(
        visible[0]
            .people
            .iter()
            .map(|p| p.name.as_str())
            .collect::<Vec<_>>(),
        ["member"]
    );
    let posted = work
        .post_room_message(&member, room.id, &message("@Bob hi"))
        .await
        .unwrap();
    assert_eq!(posted.deliveries[0].agent_id, bob);
    assert!(matches!(
        work.history(&member, bob, 0).await,
        Err(IdentityError::NotFound)
    ));

    let only_ada = RoomMembers { agents: vec![ada] };
    assert!(!work.rooms(&member).await.unwrap()[0].manage);
    assert!(matches!(
        work.set_room_members(&member, room.id, &only_ada).await,
        Err(IdentityError::Forbidden)
    ));
    identity
        .grant(
            &admin,
            &GrantRequest {
                request_id: Uuid::new_v4(),
                recipient: Recipient::User { id: member.id },
                scope: Scope::Room { id: room.id },
                capability: Capability::RoomManage,
            },
        )
        .await
        .unwrap();
    let changed = work
        .set_room_members(&member, room.id, &only_ada)
        .await
        .unwrap();
    assert!(changed.manage);
    assert_eq!(
        changed.members.iter().map(|m| m.id).collect::<Vec<_>>(),
        [ada]
    );
    assert_eq!(
        work.set_room_members(&member, room.id, &only_ada)
            .await
            .unwrap()
            .members
            .len(),
        1
    );
    assert!(matches!(
        work.set_room_members(
            &admin,
            room.id,
            &RoomMembers {
                agents: vec![Uuid::new_v4()]
            }
        )
        .await,
        Err(IdentityError::InvalidInput)
    ));
    let removed = work
        .post_room_message(&member, room.id, &message("@Bob are you there?"))
        .await
        .unwrap();
    assert_eq!(
        removed.routing, "routing",
        "a removed member is no longer mentionable"
    );
    assert!(work.history(&admin, bob, 0).await.unwrap().is_empty());
    let with_outsider = RoomMembers {
        agents: vec![ada, outsider],
    };
    let grown = work
        .set_room_members(&admin, room.id, &with_outsider)
        .await
        .unwrap();
    assert_eq!(grown.members.len(), 2);
}
