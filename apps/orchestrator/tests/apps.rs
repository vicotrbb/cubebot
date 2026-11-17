use crate::support;
use cubebot_orchestrator::{
    agents::{AgentId, Agents, CreateAgent},
    apps::{AgentState, AppState, Apps, Change, Register},
    identity::{
        Actor, Identity, IdentityError,
        access::{Capability, GrantRequest, Recipient, Scope},
    },
};
use cubebot_protocol::apps::{AppReport, Desired, Reported};
use serde_json::{Value, json};
use uuid::Uuid;

struct World {
    pool: sqlx::PgPool,
    identity: Identity,
    admin: Actor,
    ana: Actor,
    apps: Apps,
}

async fn world() -> World {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let admin = login(&identity, "admin", "administrator password").await;
    identity
        .create_account(&admin, "ana", "ana member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let ana = login(&identity, "ana", "ana member password").await;
    let apps = Apps::new(pool.clone(), identity.clone());
    World {
        pool,
        identity,
        admin,
        ana,
        apps,
    }
}

async fn login(identity: &Identity, name: &str, password: &str) -> Actor {
    let session = identity.login(name, password).await.unwrap();
    identity.authenticate(&session.token).await.unwrap()
}

async fn agent(w: &World, name: &str) -> AgentId {
    let input = CreateAgent {
        request_id: Uuid::new_v4(),
        name: name.into(),
        instructions: "Be brief".into(),
        avatar: None,
        voice: None,
    };
    let created = Agents::new(w.pool.clone()).create(&w.admin, &input).await;
    AgentId(created.unwrap().id)
}

async fn grant(w: &World, agent: AgentId, capabilities: &[Capability]) {
    for capability in capabilities {
        let grant = GrantRequest {
            request_id: Uuid::new_v4(),
            recipient: Recipient::User { id: w.ana.id },
            capability: *capability,
            scope: Scope::Conversation { id: agent.0 },
        };
        w.identity.grant(&w.admin, &grant).await.unwrap();
    }
}

fn register(input: Value) -> Register {
    let mut base = json!({
        "slug": "budget", "name": "Budget", "description": "Spending", "icon": "💰",
        "folder": "apps/budget", "command": "node server.js", "port": 3000,
        "tools": ["whoop__recovery", "whoop__sleep", "mail__send"],
    });
    for (key, value) in input.as_object().unwrap() {
        base[key] = value.clone();
    }
    serde_json::from_value(base).unwrap()
}

async fn audits(w: &World, kind: &str) -> Vec<(Option<Uuid>, Option<Uuid>, Value)> {
    sqlx::query_as("SELECT actor_id, agent_id, detail FROM audit_log WHERE kind = $1")
        .bind(kind)
        .fetch_all(&w.pool)
        .await
        .unwrap()
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_person_without_send_sees_no_apps() {
    let w = world().await;
    let scout = agent(&w, "Scout").await;
    let app = w
        .apps
        .register(scout, register(json!({})))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(app.connections, ["whoop", "mail"]);
    assert_eq!(
        (app.agent_name.as_str(), app.state),
        ("Scout", AppState::Starting)
    );

    assert!(w.apps.list(&w.ana).await.unwrap().is_empty());
    grant(&w, scout, &[Capability::ConversationRead]).await;
    assert!(w.apps.list(&w.ana).await.unwrap().is_empty());
    grant(&w, scout, &[Capability::ConversationSend]).await;
    let listed = w.apps.list(&w.ana).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].slug, "budget");
    assert_eq!(w.apps.list(&w.admin).await.unwrap().len(), 1);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn stop_and_start_need_send_and_remove_needs_manage() {
    let w = world().await;
    let scout = agent(&w, "Scout").await;
    w.apps
        .register(scout, register(json!({})))
        .await
        .unwrap()
        .unwrap();

    let refused = w.apps.set_desired(&w.ana, "budget", Desired::Stopped).await;
    assert!(matches!(refused, Err(IdentityError::NotFound)));
    grant(
        &w,
        scout,
        &[Capability::ConversationRead, Capability::ConversationSend],
    )
    .await;
    let stopped = w
        .apps
        .set_desired(&w.ana, "budget", Desired::Stopped)
        .await
        .unwrap();
    assert_eq!(stopped.state, AppState::Stopped);
    assert_eq!(
        w.apps.specs(scout).await.unwrap()[0].desired,
        Desired::Stopped
    );
    let started = w
        .apps
        .set_desired(&w.ana, "budget", Desired::Running)
        .await
        .unwrap();
    assert_eq!(started.state, AppState::Starting);
    let running = AppReport::State {
        slug: "budget".into(),
        state: Reported::Running,
        memory: Some(4096),
    };
    w.apps.report(scout, running).await.unwrap();
    let updating = AgentState {
        desired: None,
        updating: Some(true),
    };
    let app = w
        .apps
        .agent_state(scout, "budget", updating)
        .await
        .unwrap()
        .unwrap();
    assert_eq!((app.state, app.memory), (AppState::Updating, Some(4096)));
    let state = audits(&w, "app.state").await;
    assert_eq!(state.len(), 2);
    assert_eq!(state[0].0, Some(w.ana.id));

    let forbidden = w.apps.remove(&w.ana, "budget").await;
    assert!(matches!(forbidden, Err(IdentityError::Forbidden)));
    w.apps.remove(&w.admin, "budget").await.unwrap();
    assert!(w.apps.list(&w.admin).await.unwrap().is_empty());
    assert!(w.apps.specs(scout).await.unwrap().is_empty());
    assert_eq!(audits(&w, "app.remove").await.len(), 1);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn retired_agents_hide_their_apps() {
    let w = world().await;
    let scout = agent(&w, "Scout").await;
    w.apps
        .register(scout, register(json!({})))
        .await
        .unwrap()
        .unwrap();
    sqlx::query("UPDATE agents SET desired_state = 'retired' WHERE id = $1")
        .bind(scout.0)
        .execute(&w.pool)
        .await
        .unwrap();
    assert!(w.apps.list(&w.admin).await.unwrap().is_empty());
    let refused = w
        .apps
        .set_desired(&w.admin, "budget", Desired::Stopped)
        .await;
    assert!(matches!(refused, Err(IdentityError::NotFound)));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn slugs_are_unique_across_agents() {
    let w = world().await;
    let scout = agent(&w, "Scout").await;
    let other = agent(&w, "Other").await;
    w.apps
        .register(scout, register(json!({})))
        .await
        .unwrap()
        .unwrap();
    let taken = w.apps.register(other, register(json!({}))).await.unwrap();
    assert!(taken.unwrap_err().contains("taken"));

    for invalid in [
        json!({"slug": "Budget"}),
        json!({"port": 7400}),
        json!({"port": null}),
        json!({"folder": "../etc"}),
        json!({"tools": ["recovery"]}),
        json!({"name": ""}),
    ] {
        let mut fields = json!({"slug": "spare"});
        fields
            .as_object_mut()
            .unwrap()
            .extend(invalid.as_object().unwrap().clone());
        let refused = w.apps.register(other, register(fields)).await.unwrap();
        assert!(refused.is_err(), "{invalid}");
    }

    let rename: Change = serde_json::from_value(json!({"slug": "budget", "name": "Mine"})).unwrap();
    let foreign = w.apps.update(other, rename.clone()).await.unwrap();
    assert!(foreign.is_err());
    let renamed = w.apps.update(scout, rename).await.unwrap().unwrap();
    assert_eq!(
        (renamed.name.as_str(), renamed.connections.len()),
        ("Mine", 2)
    );

    let static_site = register(json!({"slug": "notes", "command": null, "port": null}));
    w.apps.register(other, static_site).await.unwrap().unwrap();
    let spec = &w.apps.specs(other).await.unwrap()[0];
    assert_eq!((spec.command.as_deref(), spec.port), (None, None));
    assert!(w.apps.unregister(scout, "notes").await.unwrap().is_err());
    w.apps.unregister(other, "notes").await.unwrap().unwrap();
    assert!(w.apps.specs(other).await.unwrap().is_empty());
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn called_reports_are_audited() {
    let w = world().await;
    let scout = agent(&w, "Scout").await;
    let other = agent(&w, "Other").await;
    w.apps
        .register(scout, register(json!({})))
        .await
        .unwrap()
        .unwrap();
    let called = |slug: &str| AppReport::Called {
        slug: slug.into(),
        tool: "whoop__recovery".into(),
        viewer: Some(w.ana.id),
        ok: true,
    };
    w.apps.report(scout, called("budget")).await.unwrap();
    w.apps.report(other, called("budget")).await.unwrap();
    let calls = audits(&w, "app.call").await;
    assert_eq!(calls.len(), 1);
    assert_eq!((calls[0].0, calls[0].1), (None, Some(scout.0)));
    assert_eq!(
        calls[0].2,
        json!({"slug": "budget", "tool": "whoop__recovery", "viewer": w.ana.id, "ok": true})
    );
}
