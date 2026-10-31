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

async fn agent(pool: &sqlx::PgPool, admin: &Actor, name: &str) -> Uuid {
    Agents::new(pool.clone())
        .create(
            admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: name.into(),
                instructions: format!("You are {name}."),
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
    for agents in [vec![], vec![Uuid::new_v4()]] {
        let invalid = CreateRoom {
            request_id: Uuid::new_v4(),
            name: "Invalid".into(),
            agents,
        };
        assert!(matches!(
            work.create_room(&admin, &invalid).await,
            Err(IdentityError::InvalidInput)
        ));
    }

    // Two distinct members, mentioned three times, plus a non-member: two deliveries, also when
    // the same request is retried concurrently.
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
    let ada_turns = work.history(&admin, ada, 0).await.unwrap();
    assert_eq!(ada_turns.len(), 1);
    assert!(
        ada_turns[0]
            .message
            .starts_with("In room Launch, admin wrote:")
    );
    let origin = ada_turns[0].origin.as_ref().unwrap();
    assert_eq!(
        (origin.kind.as_str(), origin.name.as_str()),
        ("room", "Launch")
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

    // Unaddressed: no turn until the coordinator decides. Without a provider each attempt fails,
    // and the message is reported as failed after three, never broadcast.
    let open = work
        .post_room_message(&admin, room.id, &message("Can someone check the numbers?"))
        .await
        .unwrap();
    assert_eq!(
        (open.routing.as_str(), open.deliveries.len()),
        ("routing", 0)
    );
    let connections = Connections::new(pool.clone(), Credentials::new([7; 32]));
    for _ in 0..3 {
        assert!(
            cubebot_orchestrator::work::route(&pool, &connections)
                .await
                .unwrap()
        );
    }
    assert!(
        !cubebot_orchestrator::work::route(&pool, &connections)
            .await
            .unwrap()
    );
    let messages = work.room_messages(&admin, room.id, 0).await.unwrap();
    let settled = messages.iter().find(|m| m.id == open.id).unwrap();
    assert_eq!(settled.routing, "failed");
    assert_eq!(settled.note.as_deref(), Some("provider_unavailable"));
    assert!(settled.deliveries.is_empty());
    for agent in [ada, bob] {
        assert_eq!(work.history(&admin, agent, 0).await.unwrap().len(), 1);
    }

    // Room access is its own grant; agent conversation access is not needed to use the room.
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
    assert_eq!(work.rooms(&member).await.unwrap()[0].id, room.id);
    let posted = work
        .post_room_message(&member, room.id, &message("@Bob hi"))
        .await
        .unwrap();
    assert_eq!(posted.deliveries[0].agent_id, bob);
    assert!(matches!(
        work.history(&member, bob, 0).await,
        Err(IdentityError::NotFound)
    ));

    // Membership changes need room.manage; removing a member keeps what it was given.
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
    for agents in [vec![], vec![Uuid::new_v4()]] {
        assert!(matches!(
            work.set_room_members(&admin, room.id, &RoomMembers { agents })
                .await,
            Err(IdentityError::InvalidInput)
        ));
    }
    let removed = work
        .post_room_message(&member, room.id, &message("@Bob are you there?"))
        .await
        .unwrap();
    assert_eq!(
        removed.routing, "routing",
        "a removed member is no longer mentionable"
    );
    assert_eq!(work.history(&admin, bob, 0).await.unwrap().len(), 2);
    let with_outsider = RoomMembers {
        agents: vec![ada, outsider],
    };
    let grown = work
        .set_room_members(&admin, room.id, &with_outsider)
        .await
        .unwrap();
    assert_eq!(grown.members.len(), 2);
}
