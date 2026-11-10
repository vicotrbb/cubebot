use crate::{
    memory_agent::{Model, Step, call, respond, text},
    runtime_identity::{http_server, native_chat_setup, runtime_server, spawn_native_text},
    runtime_support::fixture,
};
use axum::Router;
use cubebot_orchestrator::{
    agents::AgentId,
    identity::{Actor, Identity},
    work::{Turn, Work},
};
use serde_json::json;
use std::{sync::Arc, time::Duration};
use uuid::Uuid;

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
async fn an_agent_asks_for_an_agent_and_its_team_and_a_person_approves_them_in_order() {
    let f = fixture().await;
    let (auth, work, admin, agent) = native_chat_setup(&f, "Set up a research team").await;
    let model = Arc::new(Model::default());
    let scout = json!({"name":"Scout","instructions":"You find sources."});
    let team = json!({"name":"Research","description":"Finds sources.","instructions":"","parent":null,"reports_to":null,
        "members":[{"agent":"Scout","title":"Lead researcher","role":"Lead the research.","reports_to":null}]});
    model.turn(
        vec![
            Step::Reply(json!([
                call("team_create", team),
                call("agent_create", scout)
            ])),
            Step::Reply(json!([text("Asked for both.")])),
        ],
        json!([]),
    );
    let rename = json!({"agent":"Scout","name":"Finder","avatar":{"paint":"sky","eyes":"wink","accessory":"crown","color":null}});
    model.turn(
        vec![
            Step::Reply(json!([call("agent_update", rename)])),
            Step::Reply(json!([text("Asked to rename Scout.")])),
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

    settled(&work, &admin, agent, 1).await;
    assert!(model.requests()[0].to_string().contains("agent_create"));
    let pending = work.pending_creations(AgentId(agent)).await.unwrap();
    assert_eq!(pending.len(), 2);
    let request = |kind: &str| pending.iter().find(|p| p.kind == kind).unwrap().id;
    let created: i64 = sqlx::query_scalar!(r#"SELECT count(*) AS "n!" FROM agents"#)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(created, 1, "nothing is created before approval");

    // The team names an agent that does not exist yet, so it fails and the agent hears why.
    let failed = work
        .approve_creation(&admin, AgentId(agent), request("team"))
        .await
        .unwrap();
    assert_eq!(failed.unwrap_err(), "no agent is called Scout");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        work.history(&admin, agent, 0).await.unwrap().len(),
        1,
        "nothing is told while a request of the turn still waits"
    );

    // The console's path: list and approve over HTTP with a person's session.
    let router = cubebot_orchestrator::api::configured_router_with_credentials(
        f.pool.clone(),
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let (console, console_task) = http_server(router).await;
    let token = Identity::new(f.pool.clone())
        .login("admin", "administrator password")
        .await
        .unwrap()
        .token;
    let client = openidconnect::reqwest::Client::new();
    let listed: serde_json::Value = client
        .get(format!("{console}/api/agents/{agent}/creations"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        listed[0]["kind"], "agent",
        "only the agent is still pending"
    );
    let approved = client
        .post(format!(
            "{console}/api/agents/{agent}/creations/{}/approve",
            request("agent")
        ))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(approved.status(), 201);
    let made: serde_json::Value = approved.json().await.unwrap();
    assert_eq!(made["name"], "Scout");
    let scout: Uuid = made["id"].as_str().unwrap().parse().unwrap();
    let turns = settled(&work, &admin, agent, 2).await;
    let mut lines: Vec<&str> = turns[1].message.lines().collect();
    lines[1..].sort_unstable();
    assert_eq!(
        lines,
        [
            "A person settled every request from your earlier turn:",
            "- the agent Scout: created.",
            "- the team Research: could not be made: no agent is called Scout. Fix it and ask again.",
        ]
    );
    assert_eq!(turns[1].origin.as_ref().unwrap().kind, "approvals");
    let made = sqlx::query!(
        "SELECT created_by, desired_state FROM agents WHERE id=$1",
        scout
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(
        made.created_by, admin.id,
        "created as the person who approved it"
    );
    assert_eq!(made.desired_state, "stopped", "boots on its first message");
    assert!(
        work.approve_creation(&admin, AgentId(agent), request("agent"))
            .await
            .is_err(),
        "a settled request cannot be approved again"
    );

    let change = work.pending_creations(AgentId(agent)).await.unwrap();
    assert_eq!(change[0].kind, "change");
    let renamed = work
        .approve_creation(&admin, AgentId(agent), change[0].id)
        .await
        .unwrap();
    assert_eq!(renamed.unwrap(), (scout, "Finder".to_owned()));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        work.history(&admin, agent, 0).await.unwrap().len(),
        2,
        "an approval that is made wakes no one"
    );
    let now = sqlx::query!(
        r#"SELECT name, avatar->>'paint' AS "paint!" FROM agents WHERE id=$1"#,
        scout
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!((now.name.as_str(), now.paint.as_str()), ("Finder", "sky"));
    drop(child);
    console_task.abort();
    server.abort();
    provider_task.abort();
    f.task.abort();
}
