//! Usage and budgets, the activity inbox's read state, attachments and agent-written skills:
//! the real agent binary with a scripted model where a turn matters, real PostgreSQL throughout.
use crate::{
    memory_agent::{Model, Step, call, respond, text},
    runtime_identity::{http_server, native_chat_setup, runtime_server, spawn_native_text},
    runtime_support::fixture,
};
use axum::{
    Extension, Router,
    body::Body,
    http::{Request, StatusCode, header},
};
use cubebot_orchestrator::{
    agents::{AgentId, Agents, CreateAgent},
    identity::{Actor, Identity},
    knowledge::Knowledge,
    storage::Storage,
    work::{SendMessage, SetBudget, Turn, Work, prices_due, store_prices},
};
use cubebot_providers::Price;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tower::ServiceExt;
use uuid::Uuid;

async fn reaches(work: &Work, admin: &Actor, agent: Uuid, status: &str) -> Turn {
    for _ in 0..200 {
        let turns = work.history(admin, agent, 0).await.unwrap();
        if let Some(turn) = turns
            .into_iter()
            .last()
            .filter(|turn| turn.status == status)
        {
            return turn;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let turns = work.history(admin, agent, 0).await.unwrap();
    let last = turns.last().unwrap();
    panic!(
        "last turn is {} ({:?}), not {status}",
        last.status, last.error
    );
}

fn send(message: &str) -> SendMessage {
    SendMessage {
        request_id: Uuid::new_v4(),
        message: message.into(),
    }
}

/// `native_chat_setup`'s provider is OpenAI's `controlled-model`, here at $1/M in, $0.10/M cached
/// and $2/M out; the scripted model reports 100 in, 40 of them cached, and 20 out per response.
fn prices() -> Vec<Price> {
    vec![Price {
        model: "controlled-model".into(),
        input_per_token: 1e-6,
        cached_per_token: 1e-7,
        output_per_token: 2e-6,
    }]
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn an_agent_publishes_a_skill_itself_and_its_spend_is_priced_and_limited() {
    let f = fixture().await;
    let (auth, work, admin, agent) = native_chat_setup(&f, "Save that as a skill").await;
    let knowledge = Knowledge::new(f.pool.clone());
    let model = Arc::new(Model::default());
    let create = json!({
        "name":"disk-triage",
        "description":"Find what fills a disk. Load when a disk is nearly full.",
        "instructions":"Run du -sh * and report the largest.",
        "files":[],
    });
    model.turn(
        vec![
            Step::Reply(json!([call("skill_create", create.clone())])),
            // A retried call finds its own package; another package under the name is refused.
            Step::Reply(json!([
                call("skill_create", create.clone()),
                call(
                    "skill_create",
                    json!({"name":"disk-triage","description":"Other.","instructions":"Other.","files":[]})
                ),
                call("skill_create", json!({"name":"Bad Name","description":"x","instructions":"y","files":[]})),
            ])),
            Step::Reply(json!([text("Saved it as a skill.")])),
        ],
        json!([]),
    );
    model.turn(
        vec![Step::Reply(json!([text("Second answer.")]))],
        json!([]),
    );
    model.turn(vec![Step::Reply(json!([text("Third answer.")]))], json!([]));
    let (provider, _provider) = http_server(
        Router::new()
            .route("/responses", axum::routing::post(respond))
            .with_state(model.clone()),
    )
    .await;
    let (origin, _server) = runtime_server(auth).await;
    let _child = spawn_native_text(&origin, &provider);

    let turn = reaches(&work, &admin, agent, "complete").await;
    assert_eq!(turn.response.as_deref(), Some("Saved it as a skill."));
    let results: Vec<String> = turn
        .terminal
        .iter()
        .map(|tool| tool.outcome.as_ref().unwrap()["output"].to_string())
        .collect();
    assert!(
        results[0].contains("Published skill disk-triage"),
        "{results:?}"
    );
    assert!(results[1].contains("saved and installed"), "{results:?}");
    assert!(results[2].contains("already has a skill"), "{results:?}");
    assert!(results[3].contains("lowercase words"), "{results:?}");

    let library = knowledge.skills(&admin).await.unwrap();
    assert_eq!(library.len(), 1, "duplicates must not reach the library");
    let written = knowledge
        .skill_revision(&admin, library[0].id, 1)
        .await
        .unwrap();
    assert!(written.agent.is_some() && written.human.is_none());
    let installed = knowledge
        .installed_skills(&admin, AgentId(agent))
        .await
        .unwrap();
    assert_eq!(installed[0].name, "disk-triage");
    let activity = work.activity(&admin).await.unwrap();
    assert!(
        activity
            .iter()
            .any(|item| item.kind == "agent_published_skill" && item.turn == Some(turn.id))
    );
    let requests = model.requests();
    assert!(
        requests[0]
            .to_string()
            .contains("A skill exists only once it is published with skill_create")
    );

    // Three responses, unpriced until the price list arrives; a provider's cost is never guessed.
    let usage = work.agent_usage(&admin, AgentId(agent)).await.unwrap();
    assert!(
        usage.month.input_tokens >= 300,
        "{}",
        usage.month.input_tokens
    );
    assert!(usage.month.unpriced > 0 && usage.month.cost_usd == 0.0);
    // Only the provider in use is asked, and only once its prices are a day old.
    assert_eq!(prices_due(&f.pool).await.unwrap(), ["openai"]);
    store_prices(&f.pool, "openai", &prices()).await.unwrap();
    assert!(prices_due(&f.pool).await.unwrap().is_empty());
    let usage = work.agent_usage(&admin, AgentId(agent)).await.unwrap();
    assert_eq!(usage.month.unpriced, 0);
    let responses = usage.month.input_tokens as f64 / 100.0;
    let each = 60.0 * 1e-6 + 40.0 * 1e-7 + 20.0 * 2e-6;
    assert!((usage.month.cost_usd - responses * each).abs() < 1e-9);
    assert_eq!(usage.models[0].model, "controlled-model");
    // Another provider's price for the same model name is not this provider's.
    let other = work.agent_usage(&admin, AgentId(agent)).await.unwrap();
    store_prices(
        &f.pool,
        "openrouter",
        &[Price {
            input_per_token: 1.0,
            ..prices()[0].clone()
        }],
    )
    .await
    .unwrap();
    let after = work.agent_usage(&admin, AgentId(agent)).await.unwrap();
    assert_eq!(after.month.cost_usd, other.month.cost_usd);

    // Past the budget a new message pauses without calling the model, and raising it resumes.
    let limited = SetBudget {
        monthly_budget_usd: Some(1e-6),
    };
    work.set_budget(&admin, AgentId(agent), &limited)
        .await
        .unwrap();
    let before = model.requests().len();
    work.send(&admin, agent, &send("One more")).await.unwrap();
    let paused = reaches(&work, &admin, agent, "waiting").await;
    assert_eq!(paused.error.as_deref(), Some("budget_exhausted"));
    assert_eq!(model.requests().len(), before);
    let activity = work.activity(&admin).await.unwrap();
    assert!(
        activity
            .iter()
            .any(|item| item.text.contains("monthly budget"))
    );
    let lifted = SetBudget {
        monthly_budget_usd: None,
    };
    work.set_budget(&admin, AgentId(agent), &lifted)
        .await
        .unwrap();
    let resumed = reaches(&work, &admin, agent, "complete").await;
    assert_eq!(resumed.id, paused.id);

    work.set_budget(&admin, AgentId(agent), &limited)
        .await
        .unwrap();
    let situation = work.situation(&admin).await.unwrap();
    assert_eq!(situation.agents[0].monthly_budget_usd, Some(1e-6));
    assert!(situation.month.cost_usd > 0.0);
    assert!(!situation.recent.is_empty());
    for invalid in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        let bad = SetBudget {
            monthly_budget_usd: Some(invalid),
        };
        assert!(work.set_budget(&admin, AgentId(agent), &bad).await.is_err());
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_budget_spent_mid_turn_pauses_the_work_and_raising_it_resumes_with_its_progress() {
    let f = fixture().await;
    let (auth, work, admin, agent) = native_chat_setup(&f, "A long job").await;
    store_prices(&f.pool, "openai", &prices()).await.unwrap();
    // One scripted response costs about $0.0001, so the first spends this.
    let limited = SetBudget {
        monthly_budget_usd: Some(5e-5),
    };
    work.set_budget(&admin, AgentId(agent), &limited)
        .await
        .unwrap();
    let model = Arc::new(Model::default());
    let list = || Step::Reply(json!([call("skill_list", json!({}))]));
    let steps = (0..40).map(|_| list()).collect();
    model.turn(steps, json!([]));
    let (provider, _provider) = http_server(
        Router::new()
            .route("/responses", axum::routing::post(respond))
            .with_state(model.clone()),
    )
    .await;
    let (origin, _server) = runtime_server(auth).await;
    let _child = spawn_native_text(&origin, &provider);
    let paused = reaches(&work, &admin, agent, "waiting").await;
    assert_eq!(paused.error.as_deref(), Some("budget_exhausted"));
    assert!(paused.response.is_none());
    // The running work is told to stop, so the model is not called again and again.
    tokio::time::sleep(Duration::from_secs(4)).await;
    let settled = model.requests().len();
    assert!(settled < 40, "the agent kept spending: {settled} requests");
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(
        model.requests().len(),
        settled,
        "the agent must have stopped"
    );
    let usage = work.agent_usage(&admin, AgentId(agent)).await.unwrap();
    assert_eq!(usage.paused_turns, 1);
}

async fn names(knowledge: &Knowledge, admin: &Actor, agent: Uuid) -> Vec<String> {
    let installed = knowledge.installed_skills(admin, AgentId(agent)).await;
    installed
        .unwrap()
        .into_iter()
        .map(|skill| skill.name)
        .collect()
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn shipped_skills_reach_every_agent_once_and_uninstalling_one_sticks() {
    let f = fixture().await;
    let (_, _, admin, existing) = native_chat_setup(&f, "Hello").await;
    Knowledge::seed_shipped_skills(&f.pool).await.unwrap();
    let knowledge = Knowledge::new(f.pool.clone());
    assert_eq!(
        names(&knowledge, &admin, existing).await,
        ["coding", "design"]
    );
    let later = Agents::new(f.pool.clone())
        .create(
            &admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: "Later".into(),
                instructions: "x".into(),
                avatar: None,
            },
        )
        .await
        .unwrap()
        .id;
    assert_eq!(names(&knowledge, &admin, later).await, ["coding", "design"]);

    let library = knowledge.skills(&admin).await.unwrap();
    let design = library.iter().find(|skill| skill.name == "design").unwrap();
    knowledge
        .uninstall_skill(&admin, AgentId(existing), design.id)
        .await
        .unwrap();
    Knowledge::seed_shipped_skills(&f.pool).await.unwrap();
    assert_eq!(names(&knowledge, &admin, existing).await, ["coding"]);
    assert_eq!(knowledge.skills(&admin).await.unwrap().len(), 2);

    // A person's edit is a revision like any other; seeding again never overwrites it.
    let shipped = knowledge
        .skill_revision(&admin, design.id, 1)
        .await
        .unwrap();
    assert!(shipped.human.is_none() && shipped.dream_run.is_none() && shipped.agent.is_none());
    let mut files = shipped.files;
    files.insert("references/brand.md".into(), "Orange.".into());
    let revised = cubebot_orchestrator::knowledge::ReviseSkill {
        files,
        expected_revision: 1,
    };
    knowledge
        .revise_skill(&admin, design.id, &revised)
        .await
        .unwrap();
    Knowledge::seed_shipped_skills(&f.pool).await.unwrap();
    let library = knowledge.skills(&admin).await.unwrap();
    let design = library.iter().find(|skill| skill.name == "design").unwrap();
    assert_eq!(design.revision, 2);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn enough_finished_work_earns_a_dream_even_without_a_new_memory() {
    let f = fixture().await;
    let (auth, work, admin, agent) = native_chat_setup(&f, "One").await;
    let token = auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap()
        .token;
    let router = cubebot_orchestrator::api::runtime_router(auth);
    let dream = |run: Uuid| {
        Request::builder()
            .method("POST")
            .uri(format!("/api/runtime/dreams/{run}"))
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    };
    for count in 2..=4 {
        work.send(&admin, agent, &send(&format!("Turn {count}")))
            .await
            .unwrap();
    }
    let finish = "UPDATE chat_turns SET status='complete', response='Done', completed_at=clock_timestamp() WHERE conversation_id=$1 AND status='queued'";
    sqlx::query(finish)
        .bind(agent)
        .execute(&f.pool)
        .await
        .unwrap();
    let early = router.clone().oneshot(dream(Uuid::new_v4())).await.unwrap();
    assert_eq!(
        early.status(),
        StatusCode::CONFLICT,
        "four turns are too few to learn from"
    );
    work.send(&admin, agent, &send("Turn 5")).await.unwrap();
    sqlx::query(finish)
        .bind(agent)
        .execute(&f.pool)
        .await
        .unwrap();
    let admitted = router.oneshot(dream(Uuid::new_v4())).await.unwrap();
    let body = admitted.into_body().collect().await.unwrap().to_bytes();
    let plan: Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(plan["turns"].as_array().unwrap().len(), 5);
    assert!(plan["library"].is_array(), "{plan}");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn opening_activity_clears_the_badge_until_something_new_arrives() {
    let f = fixture().await;
    let (_, work, admin, agent) = native_chat_setup(&f, "First").await;
    let wait = |status: &'static str| {
        let pool = f.pool.clone();
        async move {
            sqlx::query("UPDATE chat_turns SET status=$1, error='provider_unavailable' WHERE conversation_id=$2 AND status='queued'")
                .bind(status)
                .bind(agent)
                .execute(&pool)
                .await
                .unwrap();
        }
    };
    wait("waiting").await;
    let unread = |items: &[cubebot_orchestrator::work::ActivityItem]| {
        items.iter().filter(|item| item.unread).count()
    };
    let items = work.activity(&admin).await.unwrap();
    assert_eq!(unread(&items), 1);
    work.read_activity(&admin).await.unwrap();
    let items = work.activity(&admin).await.unwrap();
    assert_eq!((items.len(), unread(&items)), (1, 0), "seen, not dismissed");

    // A turn created before the read but failing after it is news: it counts when it failed.
    let old = work.send(&admin, agent, &send("Second")).await.unwrap();
    work.read_activity(&admin).await.unwrap();
    wait("waiting").await;
    let items = work.activity(&admin).await.unwrap();
    let late = items.iter().find(|item| item.turn == Some(old.id)).unwrap();
    assert!(late.unread);
    assert_eq!(unread(&items), 1);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn an_attachment_is_read_through_the_reply_and_only_by_who_may_read_it() {
    let f = fixture().await;
    let (auth, work, admin, agent) = native_chat_setup(&f, "Draw me a chart").await;
    let identity = Identity::new(f.pool.clone());
    let directory = std::env::temp_dir().join(format!("cubebot-attach-{}", Uuid::new_v4()));
    let storage = Storage::start(f.pool.clone(), directory.clone(), 1 << 30)
        .await
        .unwrap();
    let token = auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap()
        .token;
    let runtime = cubebot_orchestrator::api::runtime_router(auth).layer(Extension(storage.clone()));
    let attempt: Uuid = sqlx::query_scalar(
        "SELECT attempt_id FROM runtime_session_access WHERE agent_id=$1 LIMIT 1",
    )
    .bind(agent)
    .fetch_one(&f.pool)
    .await
    .unwrap();
    let turn = work.history(&admin, agent, 0).await.unwrap()[0].id;
    sqlx::query("UPDATE chat_turns SET status='running', attempt_id=$1 WHERE id=$2")
        .bind(attempt)
        .bind(turn)
        .execute(&f.pool)
        .await
        .unwrap();
    let machine = |method: &str, uri: String, body: Body| {
        Request::builder()
            .method(method)
            .uri(uri)
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(body)
            .unwrap()
    };
    let uploaded = runtime
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/api/runtime/files?name=chart.png&request_id={}",
                    Uuid::new_v4()
                ))
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "image/png")
                .body(Body::from("png bytes"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(uploaded.status(), StatusCode::CREATED);
    let entry: Value =
        serde_json::from_slice(&uploaded.into_body().collect().await.unwrap().to_bytes()).unwrap();
    let file = entry["id"].as_str().unwrap().to_owned();
    let attach = |file: &str| {
        machine(
            "POST",
            format!("/api/runtime/turns/{turn}/attachments"),
            Body::from(json!({"file":file}).to_string()),
        )
    };
    for _ in 0..2 {
        let attached = runtime.clone().oneshot(attach(&file)).await.unwrap();
        assert_eq!(
            attached.status(),
            StatusCode::NO_CONTENT,
            "resending is fine"
        );
    }
    let foreign = runtime
        .clone()
        .oneshot(attach(&Uuid::new_v4().to_string()))
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::CONFLICT);

    let shown = &work.history(&admin, agent, 0).await.unwrap()[0].attachments;
    assert_eq!(shown.len(), 1);
    assert_eq!(
        (
            shown[0].name.as_str(),
            shown[0].content_type.as_deref(),
            shown[0].size
        ),
        ("chart.png", Some("image/png"), 9)
    );

    let console = cubebot_orchestrator::api::configured_router(f.pool.clone(), None, None)
        .await
        .unwrap()
        .layer(Extension(storage));
    let read = |session: String| {
        Request::builder()
            .uri(format!("/api/attachments/{turn}/{file}/content"))
            .header(header::AUTHORIZATION, format!("Bearer {session}"))
            .body(Body::empty())
            .unwrap()
    };
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap()
        .token;
    let response = console.clone().oneshot(read(session)).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .starts_with("attachment"),
        "the console reads the bytes; the browser never renders them in its origin"
    );
    assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "png bytes"
    );
    // Someone with no access to the conversation or a room cannot reach it.
    identity
        .create_account(
            &admin,
            "outsider",
            "outsider password",
            false,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    let session = identity
        .login("outsider", "outsider password")
        .await
        .unwrap()
        .token;
    let denied = console.oneshot(read(session)).await.unwrap();
    assert_eq!(denied.status(), StatusCode::NOT_FOUND);
    std::fs::remove_dir_all(directory).ok();
}
