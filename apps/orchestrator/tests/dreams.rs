use crate::runtime_support::fixture;
use axum::{Router, body::Body, http::Request};
use cubebot_orchestrator::{
    agents::Agents,
    connections::{ConfigureProvider, Connections, Credentials},
    identity::Identity,
    knowledge::{CreateMemory, Knowledge, ReviseMemory},
    work::{SendMessage, Work},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

struct Runtime {
    router: Router,
    token: String,
}
impl Runtime {
    async fn send(&self, method: &str, path: &str, body: Option<Value>) -> (u16, Value) {
        let request = Request::builder()
            .method(method)
            .uri(format!("/api/runtime/dreams/{path}"))
            .header("authorization", format!("Bearer {}", self.token))
            .header("content-type", "application/json");
        let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status().as_u16();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
    async fn start(&self, run: Uuid) -> (u16, Value) {
        self.send("POST", &run.to_string(), None).await
    }
    async fn commit(&self, run: Uuid, change: Value) -> u16 {
        self.send("POST", &format!("{run}/changes"), Some(change))
            .await
            .0
    }
    async fn finish(&self, run: Uuid, outcome: &str, report: &str) -> u16 {
        let end = json!({"outcome":outcome,"report":report});
        self.send("PUT", &run.to_string(), Some(end)).await.0
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn dreams_wait_for_idle_work_commit_tagged_revisions_and_never_repeat_a_change() {
    let f = fixture().await;
    let identity = Identity::new(f.pool.clone());
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    Connections::new(f.pool.clone(), Credentials::new([7; 32]))
        .configure(
            &admin,
            &ConfigureProvider {
                request_id: Uuid::new_v4(),
                expected_revision: 0,
                provider: cubebot_protocol::runtime::ProviderKind::Openai,
                model: "controlled-model".into(),
                context_window: Some(128_000),
                api_key: Some("controlled-key".into()),
                access: None,
                reasoning_effort: None,
            },
        )
        .await
        .unwrap();
    let auth = f.auth.clone().with_credentials(Credentials::new([7; 32]));
    let issued = auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let runtime = Runtime {
        router: cubebot_orchestrator::api::runtime_router(auth),
        token: issued.token,
    };
    let knowledge = Knowledge::new(f.pool.clone());

    assert_eq!(runtime.start(Uuid::new_v4()).await.0, 409);
    let memory = |content: &str| CreateMemory {
        content: content.into(),
    };
    let first = knowledge
        .create(&admin, f.agent, &memory("Deadline is Nov 22"))
        .await
        .unwrap();
    let second = knowledge
        .create(&admin, f.agent, &memory("The deadline is November 22"))
        .await
        .unwrap();

    let work = Work::new(f.pool.clone());
    let conversation = work.conversation(&admin, f.agent).await.unwrap();
    let message = SendMessage {
        request_id: Uuid::new_v4(),
        message: "hello".into(),
        attachments: Vec::new(),
        app: None,
    };
    work.send(&admin, conversation.id, &message).await.unwrap();
    assert_eq!(runtime.start(Uuid::new_v4()).await.0, 409);
    sqlx::query!(
        "
        UPDATE chat_turns
        SET status='complete',response='ok',completed_at=clock_timestamp()
        "
    )
    .execute(&f.pool)
    .await
    .unwrap();
    sqlx::query!(
        "INSERT INTO turn_events (turn_id, sequence, payload) SELECT id, 1, $1 FROM chat_turns",
        json!({"kind":"tool_call","id":Uuid::nil(),"name":"load_skill","input":{"name":"disk-triage"}}),
    )
    .execute(&f.pool)
    .await
    .unwrap();

    let run = Uuid::new_v4();
    let (status, dream) = runtime.start(run).await;
    assert_eq!(status, 200);
    assert_eq!(dream["model"], "controlled-model");
    assert_eq!(
        dream["turns"][0]["response"], "ok\n[tools used: load_skill disk-triage]",
        "a dream sees which skill a turn loaded"
    );
    assert_eq!(dream["memories"].as_array().unwrap().len(), 2);
    assert_eq!(runtime.start(run).await.0, 409, "a run id is admitted once");
    let agents = Agents::new(f.pool.clone());
    assert_eq!(
        agents.get(&admin, f.agent).await.unwrap().activity,
        "dreaming"
    );
    let merge = json!({"op":"revise","memory":first.id,"expected_revision":1,"content":"The deadline is November 22, 2026"});
    let drop = json!({"op":"forget","memory":second.id,"expected_revision":1});
    assert_eq!(runtime.commit(run, merge.clone()).await, 204);
    assert_eq!(runtime.commit(run, drop.clone()).await, 204);
    assert_eq!(
        runtime
            .finish(run, "completed", "Merged the deadline.")
            .await,
        204
    );
    assert_eq!(runtime.finish(run, "failed", "").await, 409);
    assert_eq!(runtime.commit(run, merge.clone()).await, 409);
    assert_eq!(agents.get(&admin, f.agent).await.unwrap().activity, "idle");

    assert_eq!(runtime.start(Uuid::new_v4()).await.0, 409);
    knowledge
        .revise(
            &admin,
            f.agent,
            first.id,
            &ReviseMemory {
                content: "The deadline is November 22, 2026, at noon".into(),
                expected_revision: 2,
            },
        )
        .await
        .unwrap();
    sqlx::query!(
        "
        UPDATE agents
        SET dreaming_model='dream-model'
        "
    )
    .execute(&f.pool)
    .await
    .unwrap();
    let retry = Uuid::new_v4();
    let (status, dream) = runtime.start(retry).await;
    assert_eq!(
        (status, dream["model"].as_str()),
        (200, Some("dream-model"))
    );
    assert_eq!(runtime.commit(retry, merge).await, 204);
    assert_eq!(runtime.commit(retry, drop).await, 204);
    let again = json!({"op":"remember","content":"The deadline is November 22, 2026, at noon"});
    assert_eq!(runtime.commit(retry, again).await, 204);
    assert_eq!(
        runtime.finish(retry, "completed", "Nothing new.").await,
        204
    );

    let runs = knowledge.dreams(&admin, f.agent).await.unwrap();
    assert_eq!(runs.len(), 2);
    assert_eq!(
        (runs[0].id, runs[0].outcome.as_deref()),
        (retry, Some("no_op"))
    );
    assert!(runs[0].revisions.is_empty());
    assert_eq!(runs[1].outcome.as_deref(), Some("completed"));
    assert_eq!(runs[1].report, "Merged the deadline.");
    assert!(runs[1].ended_at.is_some());
    let revised = &runs[1].revisions[0];
    assert_eq!(revised.previous.as_deref(), Some("Deadline is Nov 22"));
    assert_eq!(revised.content, "The deadline is November 22, 2026");
    assert!(runs[1].revisions[1].deleted);
    let kept: i64 = sqlx::query_scalar!(
        r#"
        SELECT count(*) AS "count!"
        FROM memory_revisions
        WHERE memory_id=$1
        "#,
        second.id,
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(kept, 2);
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_dream_grows_the_personality_and_reaches_out_whenever_it_has_a_reason() {
    let f = fixture().await;
    let identity = Identity::new(f.pool.clone());
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    Connections::new(f.pool.clone(), Credentials::new([7; 32]))
        .configure(
            &admin,
            &ConfigureProvider {
                request_id: Uuid::new_v4(),
                expected_revision: 0,
                provider: cubebot_protocol::runtime::ProviderKind::Openai,
                model: "controlled-model".into(),
                context_window: Some(128_000),
                api_key: Some("controlled-key".into()),
                access: None,
                reasoning_effort: None,
            },
        )
        .await
        .unwrap();
    let auth = f.auth.clone().with_credentials(Credentials::new([7; 32]));
    let issued = auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let runtime = Runtime {
        router: cubebot_orchestrator::api::runtime_router(auth),
        token: issued.token,
    };
    let knowledge = Knowledge::new(f.pool.clone());
    let work = Work::new(f.pool.clone());
    let conversation = work.conversation(&admin, f.agent).await.unwrap();
    let message = SendMessage {
        request_id: Uuid::new_v4(),
        message: "We ship on Friday".into(),
        attachments: Vec::new(),
        app: None,
    };
    work.send(&admin, conversation.id, &message).await.unwrap();
    sqlx::query!(
        "UPDATE chat_turns SET status='complete',response='ok',completed_at=clock_timestamp()"
    )
    .execute(&f.pool)
    .await
    .unwrap();
    let remembered = |content: &str| CreateMemory {
        content: content.into(),
    };
    let wakes = || async {
        sqlx::query_scalar!(
            "SELECT instructions FROM wakes WHERE agent_id=$1 AND set_by='agent' ORDER BY next_at",
            f.agent.0
        )
        .fetch_all(&f.pool)
        .await
        .unwrap()
    };

    knowledge
        .create(&admin, f.agent, &remembered("Ships on Friday"))
        .await
        .unwrap();
    let run = Uuid::new_v4();
    let (status, dream) = runtime.start(run).await;
    assert_eq!(status, 200);
    assert_eq!(dream["personality"], "");
    let grown = json!({"outcome":"completed","report":"Grew.","personality":"Warm and dry.","outreach":{"after_minutes":30,"intent":"Wish them luck on the launch"}});
    assert_eq!(
        runtime.send("PUT", &run.to_string(), Some(grown)).await.0,
        204
    );
    let personality = knowledge.personality(&admin, f.agent).await.unwrap();
    assert_eq!(personality.personality, "Warm and dry.");
    assert_eq!(personality.edited_by, None);
    assert_eq!(
        wakes().await,
        ["Reach out on your own: Wish them luck on the launch"]
    );
    let outcome = sqlx::query_scalar!("SELECT outcome FROM dream_runs WHERE id=$1", run)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(outcome.as_deref(), Some("completed"), "growing is a change");

    knowledge
        .create(&admin, f.agent, &remembered("Launch moved to Monday"))
        .await
        .unwrap();
    let again = Uuid::new_v4();
    let (_, dream) = runtime.start(again).await;
    assert_eq!(dream["personality"], "Warm and dry.");
    let grown = json!({"outcome":"completed","report":"Grew.","personality":"Warm, dry, patient.","outreach":{"after_minutes":5,"intent":"Ask again"}});
    assert_eq!(
        runtime.send("PUT", &again.to_string(), Some(grown)).await.0,
        204
    );
    assert_eq!(
        wakes().await,
        [
            "Reach out on your own: Ask again",
            "Reach out on your own: Wish them luck on the launch"
        ],
        "every dream with a reason reaches out"
    );
    assert_eq!(
        knowledge
            .personality(&admin, f.agent)
            .await
            .unwrap()
            .personality,
        "Warm, dry, patient."
    );

    let edit =
        |text: String| cubebot_orchestrator::knowledge::SavePersonality { personality: text };
    assert!(matches!(
        knowledge
            .save_personality(&admin, f.agent, &edit("x".repeat(801)))
            .await,
        Err(cubebot_orchestrator::identity::IdentityError::InvalidInput)
    ));
    let saved = knowledge
        .save_personality(&admin, f.agent, &edit(" Calm. ".into()))
        .await
        .unwrap();
    assert_eq!(
        (saved.personality.as_str(), saved.edited_by.as_deref()),
        ("Calm.", Some("admin"))
    );
    assert_eq!(saved.limit, 800);
    let earlier = "y".repeat(1500);
    sqlx::query!(
        "UPDATE agents SET personality=$2 WHERE id=$1",
        f.agent.0,
        earlier
    )
    .execute(&f.pool)
    .await
    .unwrap();
    let kept = knowledge.personality(&admin, f.agent).await.unwrap();
    assert_eq!(
        kept.personality, earlier,
        "a longer one written before stays readable"
    );
}
