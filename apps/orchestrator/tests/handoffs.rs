//! A login handoff (ADR 0039) end to end: the real agent binary asks for a login with a scripted
//! model, a person takes control of the desktop through the browser router against a stand-in
//! arbiter and hands it back, and the agent's next turn reads that the login completed.
use crate::{
    desktop_http::{ORIGIN, StandIn, ask, open},
    memory_agent::{Model, Step, call, respond, text},
    runtime_identity::{http_server, native_chat_setup, runtime_server, spawn_native_text},
    runtime_support::fixture,
};
use axum::Router;
use cubebot_orchestrator::{
    agents::sandbox::Desktop,
    identity::{
        Actor, Identity,
        access::{Capability, GrantRequest, Recipient, Scope},
    },
    work::{Turn, Work},
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

/// Waits until the conversation has `count` turns and the last is complete.
async fn settled(work: &Work, admin: &Actor, agent: Uuid, count: usize) -> Vec<Turn> {
    for _ in 0..200 {
        let turns = work.history(admin, agent, 0).await.unwrap();
        if turns.len() == count && turns[count - 1].status == "complete" {
            return turns;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the conversation did not reach {count} complete turns");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_person_completes_a_requested_login_and_the_next_turn_sees_it() {
    let f = fixture().await;
    let (auth, work, admin, agent) = native_chat_setup(&f, "Check my GitHub notifications").await;
    let model = Arc::new(Model::default());
    let login = json!({"site":"github.com","reason":"Read your notifications"});
    model.turn(
        vec![
            Step::Reply(json!([call("request_login", login.clone())])),
            Step::Reply(json!([text("Waiting for you to log in.")])),
        ],
        json!([]),
    );
    model.turn(
        vec![Step::Reply(json!([text("Logged in; looking now.")]))],
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

    let first = settled(&work, &admin, agent, 1).await;
    assert_eq!(
        first[0].response.as_deref(),
        Some("Waiting for you to log in.")
    );
    let pending = work
        .pending_login(cubebot_orchestrator::agents::AgentId(agent))
        .await
        .unwrap()
        .expect("a pending handoff");
    assert_eq!(
        (pending.site.as_str(), pending.reason.as_str()),
        ("github.com", "Read your notifications")
    );
    let requests = model.requests();
    assert!(
        requests[1]
            .to_string()
            .contains("A person was asked to log in on github.com"),
        "the tool told the model to end its turn"
    );

    // A controller with browser grants only: the message still reaches the agent.
    let identity = Identity::new(f.pool.clone());
    identity
        .create_account(
            &admin,
            "keyholder",
            "keyholder password",
            false,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    let session = identity
        .login("keyholder", "keyholder password")
        .await
        .unwrap();
    let keyholder = identity.authenticate(&session.token).await.unwrap().id;
    for capability in [Capability::BrowserView, Capability::BrowserControl] {
        let request = GrantRequest {
            request_id: Uuid::new_v4(),
            recipient: Recipient::User { id: keyholder },
            scope: Scope::Agent { id: agent },
            capability,
        };
        identity.grant(&admin, &request).await.unwrap();
    }
    let router = cubebot_orchestrator::api::configured_router_with_credentials(
        f.pool.clone(),
        Some(ORIGIN.into()),
        None,
        None,
    )
    .await
    .unwrap()
    .layer(axum::Extension(Desktop::new(Arc::new(StandIn))));
    let (console, console_task) = http_server(router).await;
    let mut desktop = open(&console, agent, &session.token, Some(ORIGIN))
        .await
        .unwrap();
    let taken = ask(&mut desktop, json!({"kind":"take"}), 1.0).await;
    assert_eq!(taken["mode"], "human");
    let taken_by: Option<Uuid> = sqlx::query_scalar!(
        "
        SELECT taken_by
        FROM login_handoffs
        WHERE agent_id=$1 AND status='pending'
        ",
        agent,
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(taken_by, Some(keyholder));
    let back = ask(&mut desktop, json!({"kind":"hand_back"}), 2.0).await;
    assert_eq!(back["mode"], "watch");

    let turns = settled(&work, &admin, agent, 2).await;
    assert_eq!(
        turns[1].message,
        "Login completed on github.com, take a screenshot."
    );
    assert_eq!(
        turns[1].response.as_deref(),
        Some("Logged in; looking now.")
    );
    let last = model.requests().last().unwrap().to_string();
    assert!(
        last.contains("Login completed on github.com, take a screenshot."),
        "{last}"
    );
    assert!(
        work.pending_login(cubebot_orchestrator::agents::AgentId(agent))
            .await
            .unwrap()
            .is_none()
    );
    let settled = sqlx::query!(
        "
        SELECT status, settled_by
        FROM login_handoffs
        WHERE agent_id=$1
        ",
        agent,
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(
        (settled.status.as_str(), settled.settled_by),
        ("completed", Some(keyholder))
    );

    desktop.close(None).await.unwrap();
    drop(child);
    console_task.abort();
    server.abort();
    provider_task.abort();
    f.task.abort();
}
