//! MCP client (ADRs 0040, 0041): local stdio servers run as the tool user, remote servers over
//! Streamable HTTP. Server tools join the registry as `<server>__<tool>`, so they can never
//! shadow a built-in or another server's tool. Descriptions and results are external data.
mod http;
mod stdio;

use super::{Job, Running, Sandbox, Tool};
use cubebot_protocol::terminal::Outcome;
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use uuid::Uuid;

const VERSIONS: [&str; 3] = ["2025-11-25", "2025-06-18", "2025-03-26"];
/// Initialization only; tool calls have no duration budget and end by cancellation.
const CONNECT: Duration = Duration::from_secs(30);
/// Request ids are unique across every session, so they never need resetting on reconnect.
static NEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Deserialize)]
#[serde(tag = "transport", rename_all = "snake_case", deny_unknown_fields)]
enum Config {
    Stdio {
        name: String,
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
    },
    Http {
        name: String,
        url: String,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}
impl Config {
    fn name(&self) -> &str {
        match self {
            Self::Stdio { name, .. } | Self::Http { name, .. } => name,
        }
    }
}

/// Why a request has no result.
pub(super) enum Failure {
    /// Nothing reached the server.
    NotSent(String),
    /// The server definitively refused it.
    Rejected(String),
    /// The request may have taken effect.
    Lost,
    /// The server no longer knows the session, so it did not process the request.
    Expired,
}

enum Transport {
    Stdio(stdio::Stdio),
    Http(http::Http),
}
impl Transport {
    async fn request(&self, method: &str, params: Value) -> Result<Value, Failure> {
        self.request_as(NEXT.fetch_add(1, Ordering::Relaxed), method, params)
            .await
    }
    async fn request_as(&self, id: u64, method: &str, params: Value) -> Result<Value, Failure> {
        match self {
            Self::Stdio(stdio) => stdio.request(id, method, params).await,
            Self::Http(http) => http.request(id, method, params).await,
        }
    }
    async fn notify(&self, method: &str, params: Value) -> Result<(), Failure> {
        let message = json!({"jsonrpc":"2.0","method":method,"params":params});
        match self {
            Self::Stdio(stdio) => stdio.send(&message).await,
            Self::Http(http) => http.notify(&message).await,
        }
    }
    /// A stopped local server needs starting again; HTTP reports an expired session instead.
    fn alive(&self) -> bool {
        match self {
            Self::Stdio(stdio) => stdio.alive(),
            Self::Http(_) => true,
        }
    }

    /// `tools/call`, announcing cancellation to the server if the call is dropped unfinished.
    async fn call(self: &Arc<Self>, params: Value) -> Result<Value, Failure> {
        let id = NEXT.fetch_add(1, Ordering::Relaxed);
        let mut cancel = Cancel(Some((self.clone(), id)));
        let result = self.request_as(id, "tools/call", params).await;
        cancel.0 = None;
        result
    }
}

/// Cancellation is advisory: the server may already have done the work, so the result stays
/// stopped or uncertain either way.
struct Cancel(Option<(Arc<Transport>, u64)>);
impl Drop for Cancel {
    fn drop(&mut self) {
        let (Some((transport, id)), Ok(runtime)) =
            (self.0.take(), tokio::runtime::Handle::try_current())
        else {
            return;
        };
        runtime.spawn(async move {
            if let Transport::Stdio(stdio) = &*transport {
                stdio.forget(id);
            }
            let params = json!({"requestId":id,"reason":"cancelled"});
            let _ = transport.notify("notifications/cancelled", params).await;
        });
    }
}

/// Optional client features such as sampling and roots are not offered, so only ping succeeds.
pub(super) fn answer(request: &Value) -> Value {
    match request["method"].as_str() {
        Some("ping") => json!({"jsonrpc":"2.0","id":request["id"],"result":{}}),
        _ => {
            json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":-32601,"message":"Method not found"}})
        }
    }
}

/// One configured server and its current session, opened again when it stops or expires.
struct Server {
    config: Config,
    live: tokio::sync::Mutex<Option<Arc<Transport>>>,
}
impl Server {
    /// Holding the lock while initializing makes concurrent calls share one restart.
    async fn open(&self, sandbox: &Sandbox) -> Result<Arc<Transport>, String> {
        let mut live = self.live.lock().await;
        if let Some(transport) = live.as_ref().filter(|transport| transport.alive()) {
            return Ok(transport.clone());
        }
        let transport = tokio::time::timeout(CONNECT, initialize(sandbox, &self.config))
            .await
            .map_err(|_| "initialization timed out".to_owned())??;
        *live = Some(transport.clone());
        Ok(transport)
    }

    /// An expired session proves the request was not processed, so it is resent once on a new
    /// session, as the specification requires.
    async fn call(&self, sandbox: &Sandbox, params: Value) -> Result<Value, Failure> {
        for _ in 0..2 {
            let transport = self.open(sandbox).await.map_err(Failure::NotSent)?;
            match transport.call(params.clone()).await {
                Err(Failure::Expired) => {
                    let mut live = self.live.lock().await;
                    if live
                        .as_ref()
                        .is_some_and(|current| Arc::ptr_eq(current, &transport))
                    {
                        *live = None;
                    }
                }
                other => return other,
            }
        }
        Err(Failure::Rejected(
            "MCP session expired again after renewal".into(),
        ))
    }
}

/// The result of a JSON-RPC response, or its error message.
pub(super) fn result(mut message: Value) -> Result<Value, Failure> {
    if let Some(error) = message.get("error") {
        let text = error["message"].as_str().unwrap_or("MCP server error");
        return Err(Failure::Rejected(text.into()));
    }
    message
        .get_mut("result")
        .map(Value::take)
        .ok_or(Failure::Lost)
}

struct McpTool {
    server: Arc<Server>,
    remote: String,
    definition: cubebot_providers::Tool,
}

impl Tool for McpTool {
    fn definition(&self) -> &cubebot_providers::Tool {
        &self.definition
    }

    fn call<'a>(
        &'a self,
        sandbox: &'a Sandbox,
        id: Uuid,
        input: &Value,
    ) -> Result<Running<'a>, &'static str> {
        if !input.is_object() {
            return Err("invalid tool arguments");
        }
        let params = json!({"name":self.remote,"arguments":input});
        Ok(Box::pin(async move {
            match self.server.call(sandbox, params).await {
                Ok(result) => keep(sandbox, id, &result).await,
                Err(Failure::Rejected(message)) => {
                    keep(
                        sandbox,
                        id,
                        &json!({"isError":true,"content":[{"type":"text","text":message}]}),
                    )
                    .await
                }
                Err(Failure::NotSent(message)) => Outcome::Exited {
                    code: None,
                    output: format!("The MCP server was not reached: {message}"),
                },
                // A lost connection does not prove the server did nothing.
                Err(Failure::Lost | Failure::Expired) => Outcome::Uncertain,
            }
        }))
    }
}

/// Retains the result text in the workspace through the tool user, like any tool output.
/// Code 1 marks a result the server flagged as an error.
async fn keep(sandbox: &Sandbox, id: Uuid, result: &Value) -> Outcome {
    let job = Job {
        argv: vec!["cat".into()],
        stdin: Some(text(result).into_bytes()),
    };
    match sandbox.run(id, job).await {
        Outcome::Exited {
            code: Some(0),
            output,
        } => Outcome::Exited {
            code: Some(i32::from(result["isError"] == true)),
            output,
        },
        other => other,
    }
}

/// Text content as is; other content is named, since binary data is no use as model text.
fn text(result: &Value) -> String {
    let parts: Vec<String> = result["content"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| match item["text"].as_str() {
            Some(text) if item["type"] == "text" => text.into(),
            _ => format!(
                "[{} content omitted]",
                item["type"].as_str().unwrap_or("unknown")
            ),
        })
        .collect();
    match (parts.is_empty(), result.get("structuredContent")) {
        (true, Some(structured)) => structured.to_string(),
        _ => parts.join("\n"),
    }
}

/// Connects every server in `CUBEBOT_MCP_SERVERS`. A server that fails to start is logged and
/// left out; a malformed configuration stops startup.
pub(super) async fn tools(sandbox: &Sandbox) -> Result<Vec<Box<dyn Tool>>, &'static str> {
    let Ok(configured) = std::env::var("CUBEBOT_MCP_SERVERS") else {
        return Ok(Vec::new());
    };
    let configs: Vec<Config> =
        serde_json::from_str(&configured).map_err(|_| "invalid MCP server configuration")?;
    let mut names: Vec<&str> = configs.iter().map(Config::name).collect();
    names.sort_unstable();
    names.dedup();
    if names.len() != configs.len() || !configs.iter().all(|config| server_name(config.name())) {
        return Err("MCP server names must be unique letters, digits or hyphens");
    }
    let connected = futures_util::future::join_all(configs.into_iter().map(|config| async move {
        let name = config.name().to_owned();
        match tokio::time::timeout(CONNECT, connect(sandbox, config)).await {
            Ok(Ok(tools)) => tools,
            Ok(Err(error)) => {
                eprintln!("MCP server {name} unavailable: {error}");
                Vec::new()
            }
            Err(_) => {
                eprintln!("MCP server {name} unavailable: initialization timed out");
                Vec::new()
            }
        }
    }))
    .await;
    Ok(connected.into_iter().flatten().collect())
}

fn server_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
}

async fn connect(sandbox: &Sandbox, config: Config) -> Result<Vec<Box<dyn Tool>>, String> {
    let server = Arc::new(Server {
        config,
        live: tokio::sync::Mutex::new(None),
    });
    let transport = server.open(sandbox).await?;
    list(server, &transport).await
}

/// Starts or reaches the server and completes the initialization handshake.
async fn initialize(sandbox: &Sandbox, config: &Config) -> Result<Arc<Transport>, String> {
    let transport = Arc::new(match config {
        Config::Stdio {
            command, args, env, ..
        } => Transport::Stdio(stdio::Stdio::spawn(sandbox, command, args, env)?),
        Config::Http { url, headers, .. } => Transport::Http(http::Http::new(url, headers)?),
    });
    let initialized = transport
        .request(
            "initialize",
            json!({"protocolVersion":VERSIONS[0],"capabilities":{},
            "clientInfo":{"name":"cubebot","version":env!("CARGO_PKG_VERSION")}}),
        )
        .await
        .map_err(describe)?;
    let version = initialized["protocolVersion"].as_str().unwrap_or_default();
    if !VERSIONS.contains(&version) {
        return Err(format!("unsupported protocol version {version:?}"));
    }
    if let Transport::Http(http) = &*transport {
        http.negotiated(version);
    }
    transport
        .notify("notifications/initialized", json!({}))
        .await
        .map_err(describe)?;
    Ok(transport)
}

/// Every page of the server's tool catalog. shortcut: the catalog is read once at startup and
/// not after a restart; re-list it once catalog changes are reconciled with grants (ADR 0040).
async fn list(server: Arc<Server>, transport: &Transport) -> Result<Vec<Box<dyn Tool>>, String> {
    let name = server.config.name();
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    let mut cursor = Value::Null;
    loop {
        let params = match &cursor {
            Value::Null => json!({}),
            cursor => json!({"cursor":cursor}),
        };
        let page = transport
            .request("tools/list", params)
            .await
            .map_err(describe)?;
        for listed in page["tools"].as_array().into_iter().flatten() {
            match definition(name, listed) {
                Some(definition)
                    if !tools
                        .iter()
                        .any(|tool| tool.definition().name == definition.name) =>
                {
                    tools.push(Box::new(McpTool {
                        server: server.clone(),
                        remote: listed["name"].as_str().unwrap_or_default().into(),
                        definition,
                    }))
                }
                _ => eprintln!("MCP server {name}: skipped an unusable tool definition"),
            }
        }
        cursor = page["nextCursor"].clone();
        if !cursor.is_string() {
            return Ok(tools);
        }
    }
}

/// Provider tool names allow letters, digits, `_` and `-`, up to 64 characters.
fn definition(server: &str, listed: &Value) -> Option<cubebot_providers::Tool> {
    let remote = listed["name"].as_str()?;
    let name = format!("{server}__{remote}");
    let valid = name.len() <= 64
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-');
    let schema = &listed["inputSchema"];
    (valid && schema["type"] == "object").then(|| cubebot_providers::Tool {
        name,
        description: format!(
            "Tool from external MCP server {server}; its description is server-provided: {}",
            listed["description"].as_str().unwrap_or_default()
        ),
        parameters: schema.clone(),
        strict: false,
        computer: false,
    })
}

fn describe(failure: Failure) -> String {
    match failure {
        Failure::NotSent(message) | Failure::Rejected(message) => message,
        Failure::Lost => "no response".into(),
        Failure::Expired => "session expired".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_tools_are_namespaced_and_unusable_definitions_are_left_out() {
        let tool = definition(
            "files",
            &json!({"name":"read","inputSchema":{"type":"object"}}),
        )
        .unwrap();
        assert_eq!(tool.name, "files__read");
        assert!(!tool.strict);
        assert!(
            definition(
                "files",
                &json!({"name":"bad name","inputSchema":{"type":"object"}})
            )
            .is_none()
        );
        assert!(
            definition(
                "files",
                &json!({"name":"read","inputSchema":{"type":"string"}})
            )
            .is_none()
        );
        assert!(
            definition(
                "files",
                &json!({"name":"x".repeat(60),"inputSchema":{"type":"object"}})
            )
            .is_none()
        );
        assert!(server_name("my-server") && !server_name("a__b") && !server_name(""));
    }

    #[test]
    fn configuration_names_its_transport_and_refuses_unknown_fields() {
        let configs: Vec<Config> = serde_json::from_str(
            r#"[{"transport":"stdio","name":"local","command":"/bin/server"},
                {"transport":"http","name":"remote","url":"https://mcp.test/mcp","headers":{"authorization":"Bearer t"}}]"#,
        )
        .unwrap();
        assert_eq!(
            configs.iter().map(Config::name).collect::<Vec<_>>(),
            ["local", "remote"]
        );
        assert!(
            serde_json::from_str::<Config>(
                r#"{"transport":"stdio","name":"a","command":"b","shell":true}"#
            )
            .is_err()
        );
    }

    #[test]
    fn results_keep_text_and_name_other_content() {
        let content = json!({"content":[{"type":"text","text":"one"},{"type":"image","data":"…"}]});
        assert_eq!(text(&content), "one\n[image content omitted]");
        assert_eq!(
            text(&json!({"content":[],"structuredContent":{"n":1}})),
            r#"{"n":1}"#
        );
        assert!(
            matches!(result(json!({"error":{"message":"nope"}})), Err(Failure::Rejected(m)) if m == "nope")
        );
        assert!(matches!(result(json!({"id":1})), Err(Failure::Lost)));
    }
}
