//! The real agent binary delegating through a real orchestrator and PostgreSQL, with a scripted
//! model: steering at the tool boundary, a parent yielding and resuming with a cancelled
//! child's outcome, cancellation of a whole subtree, and a failed child settling for its parent.
use crate::{
    memory_agent::{Model, Step, call, respond, text},
    runtime_identity::{http_server, native_chat_setup, runtime_server, spawn_native_text},
    runtime_support::fixture,
};
use axum::Router;
use cubebot_orchestrator::{
    agents::{Agents, CreateAgent},
    identity::Actor,
    work::{CreateRoom, SendMessage, Turn, Work},
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

async fn turn(work: &Work, admin: &Actor, conversation: Uuid, id: Uuid) -> Turn {
    let history = work.history(admin, conversation, 0).await.unwrap();
    history.into_iter().find(|turn| turn.id == id).unwrap()
}

/// Waits until the turn reaches `status`.
async fn reaches(work: &Work, admin: &Actor, conversation: Uuid, id: Uuid, status: &str) -> Turn {
    for _ in 0..200 {
        let current = turn(work, admin, conversation, id).await;
        if current.status == status {
            return current;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let current = turn(work, admin, conversation, id).await;
    panic!(
        "turn {id} is {} ({:?}), not {status}",
        current.status, current.error
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_parent_yields_to_delegated_work_and_resumes_with_its_outcome() {
    let f = fixture().await;
    let (auth, work, admin, lead) = native_chat_setup(&f, "Count the launch rows").await;
    let bee = Agents::new(f.pool.clone())
        .create(
            &admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: "Bee".into(),
                instructions: "Counts rows".into(),
            },
        )
        .await
        .unwrap()
        .id;
    work.create_room(
        &admin,
        &CreateRoom {
            request_id: Uuid::new_v4(),
            name: "Launch".into(),
            agents: vec![lead, bee],
        },
    )
    .await
    .unwrap();
    let model = Arc::new(Model::default());
    model.turn(
        vec![
            Step::Gated(json!([
                call(
                    "delegate",
                    json!({"agent":"Bee","message":"Count the rows in launch.csv"})
                ),
                call(
                    "send_message",
                    json!({"agent":"Bee","message":"FYI: launch moved a week"})
                ),
            ])),
            Step::Reply(json!([text("Waiting on Bee.")])),
        ],
        json!([]),
    );
    // Rereading its request, the resumed parent tries to delegate the same work again.
    model.turn(
        vec![
            Step::Reply(json!([call(
                "delegate",
                json!({"agent":"Bee","message":"Count the rows in launch.csv"})
            )])),
            Step::Reply(json!([text(
                "Bee's count was cancelled, so I have no count."
            )])),
        ],
        json!([]),
    );
    let (provider, provider_task) = http_server(
        Router::new()
            .route("/responses", axum::routing::post(respond))
            .with_state(model.clone()),
    )
    .await;
    let (origin, server) = runtime_server(auth).await;
    let child = spawn_native_text(&origin, &provider);

    // Steering sent while the model is thinking enters the context after the tool results.
    tokio::time::timeout(Duration::from_secs(20), model.gated.notified())
        .await
        .unwrap();
    let parent = work.history(&admin, lead, 0).await.unwrap()[0].id;
    let steer = SendMessage {
        request_id: Uuid::new_v4(),
        message: "Only count EU rows".into(),
    };
    let steering = work.steer(&admin, lead, parent, &steer).await.unwrap();
    assert_eq!(
        work.steer(&admin, lead, parent, &steer).await.unwrap().id,
        steering.id
    );
    assert!(!steering.incorporated);
    tokio::time::sleep(Duration::from_millis(2500)).await;
    model.release.notify_one();

    let yielded = reaches(&work, &admin, lead, parent, "delegated").await;
    assert!(yielded.response.is_none());
    assert_eq!(yielded.steering.len(), 1);
    assert!(yielded.steering[0].incorporated);
    assert_eq!(yielded.children.len(), 2);
    let awaited = yielded.children.iter().find(|c| c.awaited).unwrap();
    let informed = yielded.children.iter().find(|c| !c.awaited).unwrap();
    assert_eq!((awaited.agent_id, awaited.status.as_str()), (bee, "queued"));
    let bee_turns = work.history(&admin, bee, 0).await.unwrap();
    assert_eq!(bee_turns.len(), 2);
    let delegated = bee_turns.iter().find(|t| t.id == awaited.id).unwrap();
    assert!(
        delegated
            .message
            .starts_with("Delegated to you by agent Registry agent:")
    );
    assert_eq!(
        delegated.origin.as_ref().unwrap().parent_turn_id,
        Some(parent)
    );
    let requests = model.requests();
    assert!(
        requests[1].to_string().contains("Only count EU rows"),
        "steering in the second request"
    );
    assert!(requests[1].to_string().contains("Delegated to Bee as task"));

    // Steering a yielded parent reaches the task it delegated, not the message.
    let more = SendMessage {
        request_id: Uuid::new_v4(),
        message: "Exclude test rows".into(),
    };
    work.steer(&admin, lead, parent, &more).await.unwrap();
    let copied = turn(&work, &admin, bee, awaited.id).await.steering;
    assert_eq!(copied.len(), 1);
    assert!(copied[0].propagated && copied[0].message == "Exclude test rows");
    assert!(
        turn(&work, &admin, bee, informed.id)
            .await
            .steering
            .is_empty()
    );

    // Cancelling the child settles it for the parent, which resumes with the outcome.
    work.cancel(&admin, bee, awaited.id).await.unwrap();
    let resumed = reaches(&work, &admin, lead, parent, "complete").await;
    assert_eq!(
        resumed.response.as_deref(),
        Some("Bee's count was cancelled, so I have no count.")
    );
    let outcome_request = model
        .requests()
        .into_iter()
        .rev()
        .find(|request| {
            request
                .to_string()
                .contains("You already delegated the following for this request")
        })
        .unwrap()
        .to_string();
    assert!(
        outcome_request.contains(r#"\"status\":\"cancelled\""#),
        "{outcome_request}"
    );
    assert_eq!(
        turn(&work, &admin, bee, informed.id).await.status,
        "queued",
        "a message is not awaited and not cancelled with its sibling"
    );
    let last = model.requests().last().unwrap().to_string();
    assert!(
        last.contains("this exact work was already sent as task"),
        "{last}"
    );
    assert_eq!(resumed.children.len(), 2, "the repeat created no child");

    // Stopping a root stops what it delegated; the late child cannot resume it.
    model.turn(
        vec![
            Step::Reply(json!([call(
                "delegate",
                json!({"agent":"Bee","message":"Second job"})
            )])),
            Step::Reply(json!([text("Waiting again.")])),
        ],
        json!([]),
    );
    let second = work
        .send(
            &admin,
            lead,
            &SendMessage {
                request_id: Uuid::new_v4(),
                message: "Another count".into(),
            },
        )
        .await
        .unwrap()
        .id;
    let waiting = reaches(&work, &admin, lead, second, "delegated").await;
    work.cancel(&admin, lead, second).await.unwrap();
    work.cancel(&admin, lead, second).await.unwrap();
    assert_eq!(
        turn(&work, &admin, bee, waiting.children[0].id)
            .await
            .status,
        "cancelled"
    );
    assert_eq!(turn(&work, &admin, lead, second).await.status, "cancelled");

    // A failed child settles as failed with its error, and its yielded parent is queued again.
    let foreign: Uuid = sqlx::query_scalar!(
        "
        INSERT INTO chat_turns (conversation_id,actor_id,request_id,message,status)
        VALUES ($1,$2,gen_random_uuid(),'Parent elsewhere','delegated')
        RETURNING id
        ",
        bee,
        admin.id,
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    let failing: Uuid = sqlx::query_scalar!(
        "
        INSERT INTO chat_turns (conversation_id,actor_id,request_id,message,parent_turn_id,awaited)
        VALUES ($1,$2,gen_random_uuid(),'Delegated to you by agent Bee: fail',$3,true)
        RETURNING id
        ",
        lead,
        admin.id,
        foreign,
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    model.turn(vec![Step::Reject], json!([]));
    let failed = reaches(&work, &admin, lead, failing, "failed").await;
    assert_eq!(failed.error.as_deref(), Some("provider_rejected"));
    let queued = turn(&work, &admin, bee, foreign).await;
    assert_eq!(queued.status, "queued");
    assert_eq!(queued.children[0].status, "failed");

    drop(child);
    server.abort();
    provider_task.abort();
    f.task.abort();
}

/// Waits until the model has received `count` requests in all.
async fn requested(model: &Model, count: usize) {
    for _ in 0..200 {
        if model.requests().len() >= count {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!(
        "the model received {} requests, not {count}",
        model.requests().len()
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn tasks_run_in_bounded_lanes_rooms_give_context_and_replies_are_relayed() {
    let f = fixture().await;
    let (auth, work, admin, lead) = native_chat_setup(&f, "Remember the codeword mango").await;
    let bee = Agents::new(f.pool.clone())
        .create(
            &admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: "Bee".into(),
                instructions: "Reads logs".into(),
            },
        )
        .await
        .unwrap()
        .id;
    let room = work
        .create_room(
            &admin,
            &CreateRoom {
                request_id: Uuid::new_v4(),
                name: "Ops".into(),
                agents: vec![lead, bee],
            },
        )
        .await
        .unwrap();
    let post = |text: &str| SendMessage {
        request_id: Uuid::new_v4(),
        message: text.into(),
    };
    work.post_room_message(&admin, room.id, &post("@Bee look at the nginx logs"))
        .await
        .unwrap();
    let addressed = work
        .post_room_message(&admin, room.id, &post("@Registry agent summarize the room"))
        .await
        .unwrap();
    let model = Arc::new(Model::default());
    model.turn(vec![Step::Reply(json!([text("Noted: mango.")]))], json!([]));
    model.turn(
        vec![Step::Reply(json!([text("Bee is on the logs.")]))],
        json!([]),
    );
    let (provider, provider_task) = http_server(
        Router::new()
            .route("/responses", axum::routing::post(respond))
            .with_state(model.clone()),
    )
    .await;
    let (origin, server) = runtime_server(auth).await;
    let child = spawn_native_text(&origin, &provider);

    // The room delivery carries earlier room messages, which its stored request does not.
    let delivery = addressed.deliveries[0].turn;
    let delivered = reaches(&work, &admin, lead, delivery, "complete").await;
    assert!(!delivered.message.contains("nginx"));
    let roomy = model
        .requests()
        .into_iter()
        .find(|request| request.to_string().contains("summarize the room"))
        .unwrap()
        .to_string();
    assert!(
        roomy.contains("Earlier in this room") && roomy.contains("look at the nginx logs"),
        "{roomy}"
    );

    // Three delegated tasks for an agent with two task lanes: two run at once, the third waits;
    // a task's context is the task, not the main conversation.
    let mut tasks = Vec::new();
    for job in ["one", "two", "three"] {
        let parent = sqlx::query_scalar!(
            "
            INSERT INTO chat_turns (conversation_id,actor_id,request_id,message,status)
            VALUES ($1,$2,gen_random_uuid(),'Parent','delegated')
            RETURNING id
            ",
            bee,
            admin.id,
        )
        .fetch_one(&f.pool)
        .await
        .unwrap();
        tasks.push(
            sqlx::query_scalar!(
                "
                INSERT INTO chat_turns (conversation_id,actor_id,request_id,message,parent_turn_id,awaited)
                VALUES ($1,$2,gen_random_uuid(),'Delegated to you by agent Bee: job ' || $4,$3,true)
                RETURNING id
                ",
                lead,
                admin.id,
                parent,
                job,
            )
            .fetch_one(&f.pool)
            .await
            .unwrap(),
        );
    }
    let before = model.requests().len();
    model.turn(vec![Step::Gated(json!([text("done one")]))], json!([]));
    model.turn(vec![Step::Gated(json!([text("done two")]))], json!([]));
    model.turn(vec![Step::Reply(json!([text("done three")]))], json!([]));
    requested(&model, before + 2).await;
    let running: i64 = sqlx::query_scalar!(
        r#"
        SELECT count(*) AS "count!"
        FROM chat_turns
        WHERE id=ANY($1) AND status='running'
        "#,
        &tasks,
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(running, 2);
    assert_eq!(turn(&work, &admin, lead, tasks[2]).await.status, "queued");
    let task_request = model.requests()[before].to_string();
    assert!(task_request.contains("job one") || task_request.contains("job two"));
    assert!(
        !task_request.contains("mango"),
        "a task does not carry the main conversation"
    );
    model.release.notify_one();
    model.release.notify_one();
    for task in &tasks {
        reaches(&work, &admin, lead, *task, "complete").await;
    }

    // A message that expects a reply is answered back to its sender once, and the relay
    // expects none, so nothing echoes.
    let sender = sqlx::query_scalar!(
        "
        INSERT INTO chat_turns (conversation_id,actor_id,request_id,message,status,response,completed_at)
        VALUES ($1,$2,gen_random_uuid(),'Ping Registry agent','complete','sent',clock_timestamp())
        RETURNING id
        ",
        bee,
        admin.id,
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    let ping = sqlx::query_scalar!(
        "
        INSERT INTO chat_turns (conversation_id,actor_id,request_id,message,parent_turn_id,expects_reply)
        VALUES ($1,$2,gen_random_uuid(),'Message by agent Bee: ping',$3,true)
        RETURNING id
        ",
        lead,
        admin.id,
        sender,
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    model.turn(vec![Step::Reply(json!([text("pong")]))], json!([]));
    reaches(&work, &admin, lead, ping, "complete").await;
    let relays: Vec<(String, bool)> = sqlx::query!(
        "
        SELECT message,expects_reply
        FROM chat_turns
        WHERE parent_turn_id=$1
        ",
        ping,
    )
    .fetch_all(&f.pool)
    .await
    .unwrap()
    .into_iter()
    .map(|row| (row.message, row.expects_reply))
    .collect();
    assert_eq!(
        relays,
        [(
            "Reply from agent Registry agent to your message:\n\npong".to_owned(),
            false
        )]
    );

    drop(child);
    server.abort();
    provider_task.abort();
    f.task.abort();
}
