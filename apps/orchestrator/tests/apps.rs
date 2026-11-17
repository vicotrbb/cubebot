use crate::support;
use cubebot_orchestrator::work::{SendMessage, Work};
use cubebot_orchestrator::{
    agents::{AgentId, Agents, CreateAgent},
    apps::{AgentState, AppState, Apps, Change, Register},
    identity::{
        Actor, Identity, IdentityError,
        access::{Capability, GrantRequest, Recipient, Scope},
    },
};
use cubebot_protocol::apps::{AppReport, Desired, Frame, Reported, decode, encode};
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
    sqlx::query(
        "INSERT INTO connections (id, connector, slug, auth, credential, level, created_by)
         VALUES ($1, 'whoop', 'whoop', 'oauth2', '\\x00', 'read', $2)",
    )
    .bind(Uuid::new_v4())
    .bind(w.admin.id)
    .execute(&w.pool)
    .await
    .unwrap();
    let slugs: Vec<_> = app.connections.iter().map(|c| c.slug.as_str()).collect();
    assert_eq!(slugs, ["whoop", "mail"]);
    assert_eq!(
        (app.agent_name.as_str(), app.state),
        ("Scout", AppState::Unavailable)
    );

    assert!(w.apps.list(&w.ana).await.unwrap().is_empty());
    grant(&w, scout, &[Capability::ConversationRead]).await;
    assert!(w.apps.list(&w.ana).await.unwrap().is_empty());
    grant(&w, scout, &[Capability::ConversationSend]).await;
    let listed = w.apps.list(&w.ana).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].slug, "budget");
    let connectors: Vec<_> = listed[0]
        .connections
        .iter()
        .map(|c| c.connector.as_deref())
        .collect();
    assert_eq!(connectors, [Some("whoop"), None]);
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
    assert_eq!(started.state, AppState::Unavailable);
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
        json!({"folder": "."}),
        json!({"folder": "a/.git"}),
        json!({"folder": ".hidden"}),
        json!({"folder": "a\\b"}),
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
    let hidden: Change = serde_json::from_value(json!({"slug": "budget", "folder": ".x"})).unwrap();
    assert!(w.apps.update(scout, hidden).await.unwrap().is_err());
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

const ORIGIN: &str = "https://cubebot.test";

struct Relay {
    pool: sqlx::PgPool,
    apps: Apps,
    ana: Actor,
    agent: AgentId,
    base: String,
    runtime_token: String,
}

async fn relay() -> Relay {
    let f = crate::runtime_support::fixture().await;
    let identity = Identity::new(f.pool.clone());
    let admin = login(&identity, "admin", "administrator password").await;
    identity
        .create_account(&admin, "ana", "ana member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let ana = login(&identity, "ana", "ana member password").await;
    let grant = GrantRequest {
        request_id: Uuid::new_v4(),
        recipient: Recipient::User { id: ana.id },
        capability: Capability::ConversationSend,
        scope: Scope::Conversation { id: f.agent.0 },
    };
    identity.grant(&admin, &grant).await.unwrap();
    let runtime_token = f
        .auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap()
        .token;
    let apps = f.auth.apps();
    for slug in ["budget", "notes"] {
        apps.register(f.agent, register(json!({"slug": slug})))
            .await
            .unwrap()
            .unwrap();
    }
    let console = cubebot_orchestrator::api::configured_router(
        f.pool.clone(),
        Some(ORIGIN.into()),
        None,
        None,
        Some(apps.clone()),
    )
    .await
    .unwrap();
    let router = console.merge(cubebot_orchestrator::api::runtime_router(f.auth.clone()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    Relay {
        pool: f.pool,
        apps,
        ana,
        agent: f.agent,
        base,
        runtime_token,
    }
}

fn http() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap()
}

async fn fake_agent(r: &Relay, token: &str) -> Result<(), u16> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::{Message, client::IntoClientRequest};
    let url = format!("{}/api/runtime/apps/channel", r.base.replace("http", "ws"));
    let mut request = url.into_client_request().unwrap();
    request
        .headers_mut()
        .insert("authorization", format!("Bearer {token}").parse().unwrap());
    let (mut socket, _) = tokio_tungstenite::connect_async(request)
        .await
        .map_err(|error| match error {
            tokio_tungstenite::tungstenite::Error::Http(response) => response.status().as_u16(),
            _ => 0,
        })?;
    tokio::spawn(async move {
        let answer = b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok";
        while let Some(Ok(message)) = socket.next().await {
            let Message::Binary(bytes) = message else {
                continue;
            };
            if let Some(Frame::Data(stream, _)) = decode(&bytes) {
                let frames = [Frame::Data(stream, answer), Frame::Close(stream)];
                for frame in frames {
                    socket
                        .send(Message::Binary(encode(&frame).into()))
                        .await
                        .unwrap();
                }
            }
        }
    });
    while !r.apps.links().connected(r.agent) {
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    Ok(())
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_viewer_who_loses_send_is_refused_on_the_next_request() {
    let r = relay().await;
    assert_eq!(
        r.apps.list(&r.ana).await.unwrap()[0].state,
        AppState::Unavailable
    );
    assert_eq!(fake_agent(&r, "not-a-session").await, Err(401));
    fake_agent(&r, &r.runtime_token).await.unwrap();
    assert_eq!(
        r.apps.list(&r.ana).await.unwrap()[0].state,
        AppState::Starting
    );

    let cookie = "__Host-cubebot_session=ignored";
    let token = r.apps.view(&r.ana, "budget").await.unwrap();
    let page = format!("{}/api/apps/budget/v/{token}/", r.base);
    let got = http()
        .get(&page)
        .header("cookie", cookie)
        .send()
        .await
        .unwrap();
    assert_eq!(got.status(), 200);
    assert_eq!(got.headers()["access-control-allow-origin"], "*");
    assert_eq!(got.text().await.unwrap(), "ok");
    let posted = http()
        .post(format!("{page}save"))
        .header("origin", "null")
        .header("cookie", cookie)
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(posted.status(), 200);
    let preflight = http()
        .request(reqwest::Method::OPTIONS, format!("{page}save"))
        .header("origin", "null")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type")
        .send()
        .await
        .unwrap();
    assert_eq!(preflight.status(), 204);
    assert_eq!(
        preflight.headers()["access-control-allow-headers"],
        "content-type"
    );
    let bare = http()
        .get(format!("{}/api/apps/budget/v/{token}?q=1", r.base))
        .send()
        .await
        .unwrap();
    assert_eq!(bare.status(), 308);
    assert_eq!(
        bare.headers()["location"],
        format!("/api/apps/budget/v/{token}/?q=1").as_str()
    );

    sqlx::query("UPDATE access_grants SET revoked_at = now() WHERE account_id = $1")
        .bind(r.ana.id)
        .execute(&r.pool)
        .await
        .unwrap();
    let refused = http().get(&page).send().await.unwrap();
    assert_eq!(refused.status(), 404);
    let preflight = http()
        .request(reqwest::Method::OPTIONS, &page)
        .header("access-control-request-method", "POST")
        .send()
        .await
        .unwrap();
    assert_eq!(preflight.status(), 404);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_token_for_one_app_cannot_open_another() {
    let r = relay().await;
    let token = r.apps.view(&r.ana, "budget").await.unwrap();
    assert_eq!(r.apps.view(&r.ana, "budget").await.unwrap(), token);
    let base = r.base.as_str();
    let status = |path: String| async move {
        http()
            .get(format!("{base}{path}"))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    };
    assert_eq!(status(format!("/api/apps/notes/v/{token}/")).await, 404);
    assert_eq!(
        status(format!("/api/apps/budget/v/{}/", "0".repeat(64))).await,
        404
    );
    let unavailable = http()
        .get(format!("{}/api/apps/budget/v/{token}/", r.base))
        .send()
        .await
        .unwrap();
    assert_eq!(unavailable.status(), 502);
    assert!(
        unavailable
            .text()
            .await
            .unwrap()
            .contains("This app is unavailable right now")
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn minting_a_view_needs_send() {
    let r = relay().await;
    sqlx::query("UPDATE access_grants SET revoked_at = now() WHERE account_id = $1")
        .bind(r.ana.id)
        .execute(&r.pool)
        .await
        .unwrap();
    assert!(matches!(
        r.apps.view(&r.ana, "budget").await,
        Err(IdentityError::NotFound)
    ));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_retired_agents_token_is_refused() {
    let r = relay().await;
    let token = r.apps.view(&r.ana, "budget").await.unwrap();
    let page = format!("{}/api/apps/budget/v/{token}/", r.base);
    assert_eq!(http().get(&page).send().await.unwrap().status(), 502);
    sqlx::query("UPDATE agents SET desired_state = 'retired' WHERE id = $1")
        .bind(r.agent.0)
        .execute(&r.pool)
        .await
        .unwrap();
    assert_eq!(http().get(&page).send().await.unwrap().status(), 404);
    assert!(matches!(
        r.apps.view(&r.ana, "budget").await,
        Err(IdentityError::NotFound)
    ));
}

fn from_app(text: &str, app: &str) -> SendMessage {
    SendMessage {
        request_id: Uuid::new_v4(),
        message: text.into(),
        attachments: Vec::new(),
        app: Some(app.into()),
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_message_from_an_app_carries_its_origin() {
    let w = world().await;
    let scout = agent(&w, "Scout").await;
    grant(
        &w,
        scout,
        &[Capability::ConversationSend, Capability::ConversationRead],
    )
    .await;
    let _ = w.apps.register(scout, register(json!({}))).await.unwrap();
    let work = Work::new(w.pool.clone());
    work.send(&w.ana, scout.0, &from_app("How am I doing?", "budget"))
        .await
        .unwrap();
    let plain = SendMessage {
        app: None,
        ..from_app("Hello", "")
    };
    work.send(&w.ana, scout.0, &plain).await.unwrap();
    let turns = work.history(&w.ana, scout.0, 0).await.unwrap();
    let origin = turns[0].origin.as_ref().unwrap();
    assert_eq!(
        (origin.kind.as_str(), origin.name.as_str()),
        ("app", "Budget")
    );
    assert_eq!(turns[0].message, "How am I doing?");
    assert!(turns[1].origin.is_none());
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn an_app_of_another_agent_is_refused() {
    let w = world().await;
    let scout = agent(&w, "Scout").await;
    let other = agent(&w, "Other").await;
    grant(&w, scout, &[Capability::ConversationSend]).await;
    grant(&w, other, &[Capability::ConversationSend]).await;
    let _ = w.apps.register(other, register(json!({}))).await.unwrap();
    let work = Work::new(w.pool.clone());
    let refused = work.send(&w.ana, scout.0, &from_app("Hi", "budget")).await;
    assert!(matches!(refused, Err(IdentityError::NotFound)));
    let missing = work.send(&w.ana, scout.0, &from_app("Hi", "nope")).await;
    assert!(matches!(missing, Err(IdentityError::NotFound)));
}
