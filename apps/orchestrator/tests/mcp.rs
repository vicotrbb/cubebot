//! MCP servers as installation configuration (docs/adr/agents.md#local-and-remote-mcp-server-support): an administrator saves them through the
//! console API, whose answers never carry an environment or header value; per-agent grants
//! apply only to servers limited to chosen agents; and the agent's turn carries exactly the
//! servers it may use, with their secrets.
use crate::{
    runtime_identity::{
        delivery, http_server, native_chat_setup, open_channel, report, runtime_server,
    },
    runtime_support::fixture,
};
use cubebot_orchestrator::{connections::Credentials, identity::Identity};
use serde_json::{Value, json};

const TOKEN: &str = "ghp_secret-token";

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn servers_keep_their_secrets_and_reach_only_agents_with_access() {
    let f = fixture().await;
    let (auth, _work, _admin, agent) = native_chat_setup(&f, "Use GitHub").await;
    let console = cubebot_orchestrator::api::configured_router_with_credentials(
        f.pool.clone(),
        None,
        None,
        Some(Credentials::new([7; 32])),
    )
    .await
    .unwrap();
    let (console, _console_task) = http_server(console).await;
    let token = Identity::new(f.pool.clone())
        .login("admin", "administrator password")
        .await
        .unwrap()
        .token;
    let api = Console {
        http: openidconnect::reqwest::Client::new(),
        origin: console,
        token,
    };

    let github = json!({"name":"github","transport":"stdio","command":"npx",
        "args":["-y","@modelcontextprotocol/server-github"],"secrets":{"GITHUB_TOKEN":TOKEN}});
    let bad = json!({"name":"a__b","transport":"stdio","command":"npx","secrets":{}});
    let refused = api.send("POST", "/api/mcp-servers", Some(bad)).await;
    assert_eq!(refused.status(), 400);
    let created = api
        .send("POST", "/api/mcp-servers", Some(github.clone()))
        .await;
    assert_eq!(created.status(), 200);
    let created = created.text().await.unwrap();
    assert!(!created.contains(TOKEN), "{created}");
    let created: Value = serde_json::from_str(&created).unwrap();
    assert_eq!(created["secret_names"], json!(["GITHUB_TOKEN"]));
    assert_eq!(created["access"], json!({"mode":"all_agents"}));
    let server = created["id"].as_str().unwrap().to_owned();
    let twice = api.send("POST", "/api/mcp-servers", Some(github)).await;
    assert_eq!(twice.status(), 409, "names are unique");
    let stored: Vec<u8> = sqlx::query_scalar!("SELECT ciphertext FROM mcp_servers")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(
        !stored
            .windows(TOKEN.len())
            .any(|window| window == TOKEN.as_bytes())
    );

    // Shared with every agent, so a per-agent switch is refused and the agent sees it on.
    let grants = format!("/api/agents/{agent}/mcp-servers");
    let listed = api.json(&grants).await;
    assert_eq!(listed[0]["enabled"], true);
    assert_eq!(listed[0]["all_agents"], true);
    let switch = format!("{grants}/{server}");
    let set = |enabled: bool| api.send("PUT", &switch, Some(json!({"enabled":enabled})));
    assert_eq!(set(false).await.status(), 409);

    // The turn carries the server with its secret while every agent has it.
    let session = auth
        .bootstrap("projected-token", uuid::Uuid::new_v4())
        .await
        .unwrap();
    let (origin, _runtime) = runtime_server(auth.clone()).await;
    let mut channel = open_channel(&origin, &session.token).await;
    report(&mut channel, json!({"type":"pull"})).await;
    let turn = delivery(&mut channel).await;
    assert_eq!(
        turn["mcp_servers"],
        json!([{"transport":"stdio","name":"github","command":"npx",
            "args":["-y","@modelcontextprotocol/server-github"],"env":{"GITHUB_TOKEN":TOKEN}}])
    );

    // Limited to chosen agents and saved without secrets: the token stays, the agent loses it
    // until it is granted, and withdrawing it again takes it away.
    let limited = json!({"name":"github","transport":"stdio","command":"npx",
        "args":["-y","@modelcontextprotocol/server-github"],
        "access":{"mode":"explicit_grants","agents":[]}});
    let path = format!("/api/mcp-servers/{server}");
    let saved = api.send("PUT", &path, Some(limited)).await;
    assert_eq!(saved.status(), 200);
    let saved: Value = saved.json().await.unwrap();
    assert_eq!(saved["secret_names"], json!(["GITHUB_TOKEN"]));
    assert_eq!(api.json(&grants).await[0]["enabled"], false);
    assert_eq!(set(true).await.status(), 204);
    assert_eq!(api.json(&grants).await[0]["enabled"], true);
    assert_eq!(set(true).await.status(), 204, "granting twice is one grant");
    let access: Value = sqlx::query_scalar!(r#"SELECT access AS "access!" FROM mcp_servers"#)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(access["agents"], json!([agent.to_string()]));
    assert_eq!(set(false).await.status(), 204);
    assert_eq!(api.json(&grants).await[0]["enabled"], false);

    assert_eq!(api.send("DELETE", &path, None).await.status(), 204);
    assert_eq!(api.json("/api/mcp-servers").await, json!([]));
}

/// The console API as the administrator.
struct Console {
    http: openidconnect::reqwest::Client,
    origin: String,
    token: String,
}

impl Console {
    async fn send(
        &self,
        verb: &str,
        path: &str,
        body: Option<Value>,
    ) -> openidconnect::reqwest::Response {
        let url = format!("{}{path}", self.origin);
        let request = match verb {
            "POST" => self.http.post(url),
            "PUT" => self.http.put(url),
            "DELETE" => self.http.delete(url),
            _ => self.http.get(url),
        }
        .bearer_auth(&self.token);
        match body {
            Some(body) => request.json(&body),
            None => request,
        }
        .send()
        .await
        .unwrap()
    }

    async fn json(&self, path: &str) -> Value {
        self.send("GET", path, None).await.json().await.unwrap()
    }
}
