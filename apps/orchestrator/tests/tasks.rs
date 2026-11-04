//! A task from request to summary, with the real agent binary and a scripted model: the orchestrator
//! plans, the agent works from a fork of its conversation, and nothing reaches that conversation.
use crate::{
    memory_agent::{Model, Step, call, respond, text},
    runtime_identity::{http_server, native_chat_setup, runtime_server, spawn_native_text},
    runtime_support::fixture,
};
use axum::Router;
use cubebot_orchestrator::{
    agents::AgentId,
    connections::{Connections, Credentials},
    identity::{
        Identity, IdentityError,
        access::{Capability, GrantRequest, Recipient, Scope},
    },
    work::{
        SendMessage, Work,
        tasks::{CreateTask, Task, TaskSettings, advance, advance_replying},
    },
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

fn message(text: &str) -> SendMessage {
    SendMessage {
        request_id: Uuid::new_v4(),
        message: text.into(),
    }
}

async fn settled_turn(work: &Work, admin: &cubebot_orchestrator::identity::Actor, room: Uuid) {
    for _ in 0..200 {
        let messages = work.room_messages(admin, room, 0).await.unwrap();
        let open = messages
            .iter()
            .flat_map(|message| &message.deliveries)
            .any(|delivery| delivery.status != "complete");
        if !open
            && messages
                .iter()
                .any(|message| !message.deliveries.is_empty())
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the task's turn did not complete");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_task_runs_from_a_fork_of_the_agent_and_leaves_its_conversation_alone() {
    let f = fixture().await;
    let (auth, work, admin, agent) = native_chat_setup(&f, "Main hello").await;
    let connections = Connections::new(f.pool.clone(), Credentials::new([7; 32]));
    let model = Arc::new(Model::default());
    model.turn(vec![Step::Reply(json!([text("Main answer")]))], json!([]));
    model.turn(
        vec![
            Step::Reply(json!([
                call(
                    "task_item",
                    json!({"item":1,"status":"done","note":"two found"})
                ),
                call("task_item", json!({"item":99,"status":"done","note":null})),
                call(
                    "task_message",
                    json!({"message":"Found two venues.","to":[]})
                ),
            ])),
            Step::Reply(json!([text("Task answer")])),
        ],
        json!([]),
    );
    model.turn(vec![Step::Reply(json!([text("Later answer")]))], json!([]));
    let (provider, _provider) = http_server(
        Router::new()
            .route("/responses", axum::routing::post(respond))
            .with_state(model.clone()),
    )
    .await;
    let (origin, _server) = runtime_server(auth).await;
    let _child = spawn_native_text(&origin, &provider);
    let main = async {
        for _ in 0..200 {
            let turns = work.history(&admin, agent, 0).await.unwrap();
            if turns.last().is_some_and(|turn| turn.status == "complete") {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the main turn did not complete");
    };
    main.await;

    // The request is recorded without a model; a repeat is the same task.
    let create = CreateTask {
        request_id: Uuid::new_v4(),
        request: "Compare two venues".into(),
    };
    let task = work.create_task(&admin, &create).await.unwrap();
    assert_eq!(work.create_task(&admin, &create).await.unwrap().id, task.id);
    assert_eq!((task.status.as_str(), task.reading), ("working", true));
    assert!(task.title.is_none());
    assert!(
        work.rooms(&admin)
            .await
            .unwrap()
            .iter()
            .all(|room| room.id != task.room_id),
        "a task's room stays off the room list"
    );

    let plan = json!({"title":"Venue comparison","say":"Ayla takes this.","action":"assign","assignments":[
        {"agent":agent,"brief":"Compare the venues.","items":["Find them","Write it up"]},
        {"agent":Uuid::new_v4(),"brief":"An agent that does not exist","items":["x"]},
    ]});
    assert!(
        advance_replying(&f.pool, &connections, &plan.to_string())
            .await
            .unwrap()
    );
    let planned = work.task(&admin, task.id).await.unwrap();
    assert_eq!(planned.title.as_deref(), Some("Venue comparison"));
    assert!(!planned.reading);
    assert_eq!(planned.items.len(), 2, "the stranger gets nothing");
    assert_eq!(planned.agents.len(), 1);

    settled_turn(&work, &admin, task.room_id).await;
    let messages = work.room_messages(&admin, task.room_id, 0).await.unwrap();
    assert!(messages[1].orchestrator && messages[1].author == "Orchestrator");
    let delivery = &messages[1].deliveries[0];
    assert_eq!(delivery.response.as_deref(), Some("Task answer"));
    assert_eq!(delivery.terminal.len(), 3, "its work shows in the room");
    let spoken = messages
        .iter()
        .find(|message| message.author_agent_id == Some(agent))
        .expect("the agent spoke in the room");
    assert_eq!(spoken.message, "Found two venues.");
    let refused = delivery.terminal[1].outcome.as_ref().unwrap()["output"].to_string();
    assert!(refused.contains("not one of yours"), "{refused}");
    let progressed = work.task(&admin, task.id).await.unwrap();
    assert_eq!(progressed.items[0].status, "done");
    assert_eq!(progressed.items[0].note.as_deref(), Some("two found"));
    assert_eq!(progressed.items[1].status, "todo");

    // The fork: the task's turn saw the agent's conversation and the task's tools, not its own
    // collaboration tools.
    let requests = model.requests();
    let task_request = requests
        .iter()
        .find(|request| request.to_string().contains("Compare the venues."))
        .unwrap();
    assert!(task_request["input"].to_string().contains("Main answer"));
    let tools = task_request["tools"].to_string();
    assert!(tools.contains("task_message") && !tools.contains("\"send_message\""));

    // Nothing came back: the conversation holds only its own turn, and its next turn knows nothing.
    let main_turns = work.history(&admin, agent, 0).await.unwrap();
    assert_eq!(main_turns.len(), 1);
    let conversation = work.conversation(&admin, AgentId(agent)).await.unwrap();
    work.send(&admin, conversation.id, &message("Main again"))
        .await
        .unwrap();
    for _ in 0..200 {
        let turns = work.history(&admin, agent, 0).await.unwrap();
        if turns.len() == 2 && turns[1].status == "complete" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let later = model
        .requests()
        .into_iter()
        .find(|request| request["input"].to_string().contains("Main again"))
        .unwrap();
    assert!(!later["input"].to_string().contains("Task answer"));
    assert!(!later["input"].to_string().contains("Found two venues"));

    // Agents settled, so the orchestrator is due; it cannot finish past open work, and finishing
    // closes the items the agent left.
    let finish = json!({"say":"Done: two venues compared.","action":"finish"}).to_string();
    assert!(
        advance_replying(&f.pool, &connections, &finish)
            .await
            .unwrap()
    );
    let done: Task = work.task(&admin, task.id).await.unwrap();
    assert_eq!(done.status, "done");
    assert_eq!(done.items[1].status, "done");
    assert!(
        !advance_replying(&f.pool, &connections, &finish)
            .await
            .unwrap()
    );

    // Writing brings it back; an unaddressed message waits for the orchestrator, whose question
    // puts it in the person's hands.
    let again = work
        .post_room_message(&admin, task.room_id, &message("Add a third?"))
        .await
        .unwrap();
    assert_eq!(again.routing, "routing");
    assert_eq!(work.task(&admin, task.id).await.unwrap().status, "working");
    let ask = json!({"say":"Which city?","action":"ask"}).to_string();
    assert!(advance_replying(&f.pool, &connections, &ask).await.unwrap());
    let asked = work.task(&admin, task.id).await.unwrap();
    assert_eq!(asked.status, "needs_you");
    assert!(!asked.reading);

    // An answer the model garbles three times leaves the person told, not the task stuck.
    work.post_room_message(&admin, task.room_id, &message("Lisbon"))
        .await
        .unwrap();
    let garbled = [
        advance_replying(&f.pool, &connections, "not json")
            .await
            .unwrap(),
        advance_replying(&f.pool, &connections, "not json")
            .await
            .unwrap(),
        advance_replying(&f.pool, &connections, "not json")
            .await
            .unwrap(),
    ];
    assert_eq!(garbled, [false, false, true]);
    let told = work.task(&admin, task.id).await.unwrap();
    assert_eq!(told.status, "needs_you");
    assert!(!told.reading);

    let stopped = work.stop_task(&admin, task.id).await.unwrap();
    assert_eq!(stopped.status, "stopped");
    assert!(!advance(&f.pool, &connections).await.unwrap());
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn tasks_belong_to_their_person_and_reach_only_agents_they_may_use() {
    let f = fixture().await;
    let (_auth, work, admin, agent) = native_chat_setup(&f, "hello").await;
    let identity = Identity::new(f.pool.clone());
    let mut people = Vec::new();
    for login in ["owner", "other"] {
        identity
            .create_account(&admin, login, "member password", false, Uuid::new_v4())
            .await
            .unwrap();
        let session = identity.login(login, "member password").await.unwrap();
        people.push(identity.authenticate(&session.token).await.unwrap());
    }
    let (owner, other) = (&people[0], &people[1]);
    let request = |text: &str| CreateTask {
        request_id: Uuid::new_v4(),
        request: text.into(),
    };

    // No agent to hand it to, and blank requests, are refused.
    assert!(matches!(
        work.create_task(owner, &request("Do it")).await,
        Err(IdentityError::Conflict)
    ));
    assert!(matches!(
        work.create_task(&admin, &request("  ")).await,
        Err(IdentityError::InvalidInput)
    ));
    identity
        .grant(
            &admin,
            &GrantRequest {
                request_id: Uuid::new_v4(),
                recipient: Recipient::User { id: owner.id },
                scope: Scope::Conversation { id: agent },
                capability: Capability::ConversationSend,
            },
        )
        .await
        .unwrap();
    let task = work.create_task(owner, &request("Do it")).await.unwrap();

    // Its owner and administrators see it; someone else cannot find it or its room.
    assert_eq!(work.tasks(owner).await.unwrap().len(), 1);
    assert_eq!(work.tasks(&admin).await.unwrap().len(), 1);
    assert!(work.tasks(other).await.unwrap().is_empty());
    assert!(matches!(
        work.task(other, task.id).await,
        Err(IdentityError::NotFound)
    ));
    assert!(matches!(
        work.stop_task(other, task.id).await,
        Err(IdentityError::NotFound)
    ));
    assert!(matches!(
        work.room_messages(other, task.room_id, 0).await,
        Err(IdentityError::NotFound)
    ));
    assert!(work.room_messages(owner, task.room_id, 0).await.is_ok());
    // An administrator's mention still carries the owner's authority, not theirs.
    let name: String = sqlx::query_scalar("SELECT name FROM agents WHERE id=$1")
        .bind(agent)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let posted = work
        .post_room_message(&admin, task.room_id, &message(&format!("@{name} go")))
        .await
        .unwrap();
    let delivered = posted.deliveries[0].turn;
    let actor: Uuid = sqlx::query_scalar("SELECT actor_id FROM chat_turns WHERE id=$1")
        .bind(delivered)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(actor, owner.id);

    // Using the agent shows it is busy, but not on what, and gives no hold on the task's turns.
    for capability in [Capability::ConversationRead, Capability::ConversationSend] {
        identity
            .grant(
                &admin,
                &GrantRequest {
                    request_id: Uuid::new_v4(),
                    recipient: Recipient::User { id: other.id },
                    scope: Scope::Conversation { id: agent },
                    capability,
                },
            )
            .await
            .unwrap();
    }
    let seen = work.situation(other).await.unwrap();
    let busy = seen
        .work
        .iter()
        .find(|item| item.turn == delivered)
        .unwrap();
    assert_eq!(busy.message, "On a task");
    assert!(busy.room.is_none());
    assert!(matches!(
        work.cancel(other, agent, delivered).await,
        Err(IdentityError::NotFound)
    ));
    sqlx::query("UPDATE chat_turns SET status='waiting', error='provider_unavailable' WHERE id=$1")
        .bind(delivered)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(
        work.activity(other)
            .await
            .unwrap()
            .iter()
            .all(|item| item.turn != Some(delivered)),
        "a stuck task turn is for its owner, not everyone who uses the agent"
    );
    assert_eq!(
        work.activity(owner)
            .await
            .unwrap()
            .iter()
            .find(|item| item.turn == Some(delivered))
            .unwrap()
            .task,
        Some(task.id)
    );
    assert!(work.stop_task(owner, task.id).await.is_ok());

    // Settings are for administrators, and a stale save is refused.
    assert!(work.task_settings(owner).await.is_err());
    let settings = work.task_settings(&admin).await.unwrap();
    assert_eq!((settings.max_agents, settings.max_rounds), (3, 3));
    let change = |revision| TaskSettings {
        provider: None,
        model: None,
        max_agents: 5,
        max_rounds: 2,
        instructions: "Ask before spending.".into(),
        revision,
    };
    let saved = work
        .save_task_settings(&admin, &change(settings.revision))
        .await
        .unwrap();
    assert_eq!(saved.max_agents, 5);
    assert_eq!(saved.revision, settings.revision + 1);
    assert!(matches!(
        work.save_task_settings(&admin, &change(settings.revision))
            .await,
        Err(IdentityError::Conflict)
    ));
    assert!(matches!(
        work.save_task_settings(owner, &change(saved.revision))
            .await,
        Err(IdentityError::Forbidden | IdentityError::NotFound)
    ));
}
