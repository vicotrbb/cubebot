use crate::{rooms::agent, support};
use cubebot_orchestrator::{
    identity::{Identity, IdentityError},
    work::{CreateTeam, RoomMembers, SaveMember, SaveTeam, SendMessage, UpdateTeam, Work},
};
use uuid::Uuid;

fn member(agent: Uuid, title: &str, reports_to: Option<Uuid>) -> SaveMember {
    SaveMember {
        agent,
        title: title.into(),
        role: String::new(),
        reports_to,
    }
}

fn save(name: &str, parent: Option<(Uuid, Uuid)>, members: Vec<SaveMember>) -> SaveTeam {
    SaveTeam {
        name: name.into(),
        description: String::new(),
        instructions: String::new(),
        parent: parent.map(|(team, _)| team),
        reports_to: parent.map(|(_, boss)| boss),
        members,
    }
}

fn agents_of(room: &cubebot_orchestrator::work::Room) -> Vec<Uuid> {
    let mut ids: Vec<Uuid> = room.members.iter().map(|member| member.id).collect();
    ids.sort();
    ids
}

fn sorted(mut ids: Vec<Uuid>) -> Vec<Uuid> {
    ids.sort();
    ids
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn team_rooms_follow_rosters_and_nesting_and_a_deleted_team_archives_its_room() {
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
    let (cto, lead, dev) = (
        agent(&pool, &admin, "Cyd").await,
        agent(&pool, &admin, "Lia").await,
        agent(&pool, &admin, "Dev").await,
    );
    let work = Work::new(pool.clone());
    let create = CreateTeam {
        request_id: Uuid::new_v4(),
        team: save("Leadership", None, vec![member(cto, "CTO", None)]),
    };
    let top = work.create_team(&admin, &create).await.unwrap();
    assert_eq!(work.create_team(&admin, &create).await.unwrap().id, top.id);
    assert_eq!(top.revision, 1);
    let engineering = work
        .create_team(
            &admin,
            &CreateTeam {
                request_id: Uuid::new_v4(),
                team: save(
                    "Engineering",
                    Some((top.id, cto)),
                    vec![
                        member(lead, "Engineering manager", None),
                        member(dev, "Backend engineer", Some(lead)),
                    ],
                ),
            },
        )
        .await
        .unwrap();
    let duplicate = CreateTeam {
        request_id: Uuid::new_v4(),
        team: save(" leadership", None, vec![member(dev, "Dev", None)]),
    };
    assert!(matches!(
        work.create_team(&admin, &duplicate).await,
        Err(IdentityError::InvalidInput)
    ));

    let rooms = work.rooms(&admin).await.unwrap();
    let room_of = |id: Uuid| rooms.iter().find(|room| room.id == id).unwrap();
    assert_eq!(room_of(top.room).team, Some(top.id));
    assert_eq!(agents_of(room_of(top.room)), sorted(vec![cto, lead]));
    assert_eq!(
        agents_of(room_of(engineering.room)),
        sorted(vec![lead, dev])
    );
    assert!(matches!(
        work.set_room_members(&admin, top.room, &RoomMembers { agents: vec![cto] })
            .await,
        Err(IdentityError::Forbidden)
    ));

    // Making Dev the manager moves Lia under it, and Dev into the parent's room.
    let changed = save(
        "Platform",
        Some((top.id, cto)),
        vec![
            member(dev, "Engineering manager", None),
            member(lead, "Staff engineer", Some(dev)),
        ],
    );
    let stale = UpdateTeam {
        expected_revision: engineering.revision + 1,
        team: changed.clone(),
    };
    assert!(matches!(
        work.update_team(&admin, engineering.id, &stale).await,
        Err(IdentityError::Conflict)
    ));
    let platform = work
        .update_team(
            &admin,
            engineering.id,
            &UpdateTeam {
                expected_revision: engineering.revision,
                team: changed.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(platform.revision, engineering.revision + 1);
    let rooms = work.rooms(&admin).await.unwrap();
    let room_of = |id: Uuid| rooms.iter().find(|room| room.id == id).unwrap();
    assert_eq!(room_of(platform.room).name, "Platform");
    assert_eq!(agents_of(room_of(top.room)), sorted(vec![cto, dev]));

    // A team cannot sit under a team below it.
    let mut looping = save(
        "Leadership",
        Some((platform.id, dev)),
        vec![member(cto, "CTO", None)],
    );
    looping.parent = Some(platform.id);
    assert!(matches!(
        work.update_team(
            &admin,
            top.id,
            &UpdateTeam {
                expected_revision: top.revision,
                team: looping,
            },
        )
        .await,
        Err(IdentityError::InvalidInput)
    ));

    assert!(matches!(
        work.delete_team(&admin, top.id, top.revision).await,
        Err(IdentityError::Conflict)
    ));
    work.delete_team(&admin, platform.id, platform.revision)
        .await
        .unwrap();
    let rooms = work.rooms(&admin).await.unwrap();
    assert!(
        rooms
            .iter()
            .any(|room| room.id == platform.room && room.archived && room.members.is_empty())
    );
    assert_eq!(
        agents_of(rooms.iter().find(|room| room.id == top.room).unwrap()),
        vec![cto]
    );
    assert!(matches!(
        work.post_room_message(
            &admin,
            platform.room,
            &SendMessage {
                request_id: Uuid::new_v4(),
                message: "hello".into(),
                attachments: Vec::new(),
            },
        )
        .await,
        Err(IdentityError::Forbidden)
    ));
    let teams = work.teams(&admin).await.unwrap();
    assert_eq!(teams.len(), 1);
    assert_eq!(teams[0].members[0].title, "CTO");

    let research = work
        .create_team(
            &admin,
            &CreateTeam {
                request_id: Uuid::new_v4(),
                team: save("Research", Some((top.id, cto)), vec![]),
            },
        )
        .await
        .unwrap();
    assert!(research.members.is_empty());
    let leadership = work.teams(&admin).await.unwrap();
    let leadership = leadership.iter().find(|team| team.id == top.id).unwrap();
    assert!(matches!(
        work.update_team(
            &admin,
            top.id,
            &UpdateTeam {
                expected_revision: leadership.revision,
                team: save("Leadership", None, vec![]),
            },
        )
        .await,
        Err(IdentityError::InvalidInput)
    ));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn an_administrator_deletes_a_room_but_not_a_live_team_room() {
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
    let work = Work::new(pool.clone());
    let team = work
        .create_team(
            &admin,
            &CreateTeam {
                request_id: Uuid::new_v4(),
                team: save("Ops", None, vec![member(ada, "Lead", None)]),
            },
        )
        .await
        .unwrap();
    assert!(matches!(
        work.delete_room(&admin, team.room).await,
        Err(IdentityError::Conflict)
    ));
    let room = work
        .create_room(
            &admin,
            &cubebot_orchestrator::work::CreateRoom {
                request_id: Uuid::new_v4(),
                name: "Scratch".into(),
                agents: vec![ada],
            },
        )
        .await
        .unwrap();
    assert!(work.delete_room(&admin, room.id).await.unwrap().archived);
    assert!(work.delete_room(&admin, room.id).await.unwrap().archived);
    let refused = work
        .post_room_message(
            &admin,
            room.id,
            &SendMessage {
                request_id: Uuid::new_v4(),
                message: "Still here?".into(),
                attachments: Vec::new(),
            },
        )
        .await;
    assert!(refused.is_err(), "nobody writes in a deleted room");
    assert!(matches!(
        work.delete_room(&admin, Uuid::new_v4()).await,
        Err(IdentityError::NotFound)
    ));
}
