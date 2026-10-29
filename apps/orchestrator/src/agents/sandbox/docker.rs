//! Cluster-less sandboxes through the `docker` CLI: one container per attempt, one volume per agent.
//!
//! Each container is a pod attempt in namespace `docker`, so sessions, drains and epochs work as on
//! Kubernetes. The attempt is recorded before the container is created; the epoch in the container
//! name makes creation idempotent, so a lost reply is recovered by lookup, never by a second run.
use super::{Backend, ExecChannel, Sandbox, Status};
use crate::agents::{
    AgentId,
    kubernetes::{AttemptId, Error},
    runtime::{sandbox_digest, sandbox_token},
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{io::Read, process::Stdio};
use tokio::process::Command;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DockerConfig {
    image: String,
    backend: Backend,
    orchestrator_origin: String,
    #[serde(default)]
    terminal_enabled: bool,
}
impl DockerConfig {
    pub fn parse(input: &str) -> Result<Self, Error> {
        let config: Self = serde_json::from_str(input).map_err(|_| Error::InvalidConfiguration)?;
        // Values become docker arguments; a leading '-' would be read as a flag.
        let argument = |value: &str| {
            !value.is_empty()
                && value.len() <= 512
                && !value.starts_with('-')
                && !value
                    .bytes()
                    .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        };
        // The agent accepts HTTPS, or HTTP only to loopback (reached through host networking).
        let origin = openidconnect::reqwest::Url::parse(&config.orchestrator_origin)
            .map_err(|_| Error::InvalidConfiguration)?;
        // A kvm sandbox on host networking would redirect the host's own interface to its guest.
        if !argument(&config.image)
            || !argument(&config.orchestrator_origin)
            || !(origin.scheme() == "https" || (origin.scheme() == "http" && loopback(&origin)))
            || (config.backend == Backend::Kvm && loopback(&origin))
        {
            return Err(Error::InvalidConfiguration);
        }
        Ok(config)
    }
    pub fn environment() -> Result<Option<Self>, Error> {
        let Some(path) = std::env::var_os("CUBEBOT_DOCKER_CONFIG") else {
            return Ok(None);
        };
        let mut input = String::new();
        std::fs::File::open(path)
            .and_then(|file| file.take(65537).read_to_string(&mut input))
            .map_err(|_| Error::InvalidConfiguration)?;
        if input.len() > 65536 {
            return Err(Error::InvalidConfiguration);
        }
        Self::parse(&input).map(Some)
    }
}
fn loopback(origin: &openidconnect::reqwest::Url) -> bool {
    origin.host_str().is_some_and(|host| {
        host.trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
    })
}

pub struct Docker {
    pool: PgPool,
    config: DockerConfig,
    program: String,
}
#[derive(sqlx::FromRow)]
struct Attempt {
    id: AttemptId,
    name: String,
    target_generation: i64,
    phase: String,
}
impl Docker {
    pub fn new(pool: PgPool, config: DockerConfig) -> Self {
        Self {
            pool,
            config,
            program: "docker".into(),
        }
    }
    /// Tests point this at a fake CLI.
    pub fn with_program(mut self, program: &str) -> Self {
        self.program = program.into();
        self
    }
    async fn docker(&self, args: &[&str]) -> Result<String, Error> {
        let output = Command::new(&self.program)
            .args(args)
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(Error::Sandbox)?;
        if !output.status.success() {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            return Err(Error::Sandbox(std::io::Error::other(message)));
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    }
    /// The container's `State`, or `None` when it does not exist.
    async fn inspect(&self, name: &str) -> Result<Option<Value>, Error> {
        match self
            .docker(&["container", "inspect", "--format", "{{json .State}}", name])
            .await
        {
            Ok(state) => serde_json::from_str(&state)
                .map(Some)
                .map_err(|_| Error::InvalidConfiguration),
            Err(Error::Sandbox(error)) if error.to_string().contains("No such container") => {
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
    async fn current(&self, agent: AgentId) -> Result<Option<Attempt>, Error> {
        Ok(sqlx::query_as("SELECT id,name,target_generation,phase FROM agent_resources WHERE agent_id = $1 AND kind = 'pod' AND namespace = 'docker' AND phase <> 'stopped'")
            .bind(agent).fetch_optional(&self.pool).await?)
    }
    async fn intent(&self, agent: AgentId) -> Result<(i64, i64), Error> {
        sqlx::query_as("SELECT revision,restart_generation FROM agents WHERE id = $1")
            .bind(agent)
            .fetch_optional(&self.pool)
            .await?
            .ok_or(Error::Conflict)
    }
    async fn record(&self, agent: AgentId) -> Result<Attempt, Error> {
        let (revision, generation) = self.intent(agent).await?;
        let epoch: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(epoch),0) + 1 FROM agent_resources WHERE agent_id = $1 AND kind = 'pod'")
            .bind(agent).fetch_one(&self.pool).await?;
        let name = format!("cubebot-agent-{}-{epoch}", agent.0);
        let manifest =
            json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":name,"namespace":"docker"}});
        let id = AttemptId(Uuid::new_v4());
        sqlx::query("INSERT INTO agent_resources (id,agent_id,kind,epoch,target_generation,definition_revision,namespace,name,manifest,source_manifest,phase,prepared) VALUES ($1,$2,'pod',$3,$4,$5,'docker',$6,$7,$7,'creating',true)")
            .bind(id).bind(agent).bind(epoch).bind(generation).bind(revision).bind(&name).bind(&manifest).execute(&self.pool).await?;
        Ok(Attempt {
            id,
            name,
            target_generation: generation,
            phase: "creating".into(),
        })
    }
    /// Creates the recorded container if it does not exist yet and binds its ID.
    async fn bind(&self, agent: AgentId, attempt: &Attempt) -> Result<(), Error> {
        if self.inspect(&attempt.name).await?.is_none() {
            let token = sandbox_token();
            sqlx::query("INSERT INTO docker_sandbox_tokens (attempt_id,token_hash) VALUES ($1,$2) ON CONFLICT (attempt_id) DO UPDATE SET token_hash = EXCLUDED.token_hash")
                .bind(attempt.id).bind(sandbox_digest(&token)?).execute(&self.pool).await?;
            self.create_container(agent, &attempt.name, &token).await?;
        }
        let uid = self
            .docker(&["container", "inspect", "--format", "{{.Id}}", &attempt.name])
            .await?;
        sqlx::query("UPDATE agent_resources SET uid = $2,resource_version = '1',phase = 'bound',updated_at = clock_timestamp() WHERE id = $1 AND phase = 'creating'")
            .bind(attempt.id).bind(uid).execute(&self.pool).await?;
        Ok(())
    }
    async fn create_container(&self, agent: AgentId, name: &str, token: &str) -> Result<(), Error> {
        let label = format!("cubebot.dev/agent={}", agent.0);
        let mount = format!(
            "type=volume,source={},target=/var/lib/cubebot",
            volume(agent)
        );
        let origin = format!(
            "CUBEBOT_ORCHESTRATOR_ORIGIN={}",
            self.config.orchestrator_origin
        );
        let token = format!("CUBEBOT_BOOTSTRAP_TOKEN={token}");
        let mut args = vec![
            "container",
            "create",
            "--name",
            name,
            "--label",
            &label,
            "--read-only",
            "--tmpfs",
            "/tmp",
            "--cap-drop",
            "ALL",
            "--security-opt",
            "no-new-privileges",
            "--mount",
            &mount,
            "--env",
            &origin,
            "--env",
            &token,
            "--env",
            "CUBEBOT_JOURNAL_DIRECTORY=/var/lib/cubebot/runtime",
            "--env",
            "CUBEBOT_WORKSPACE_DIRECTORY=/var/lib/cubebot/workspace",
        ];
        let origin = openidconnect::reqwest::Url::parse(&self.config.orchestrator_origin)
            .map_err(|_| Error::InvalidConfiguration)?;
        if loopback(&origin) {
            args.extend(["--network", "host"]);
        }
        if self.config.terminal_enabled {
            // shortcut: the process backend cannot switch users, so tools run as the agent's own
            // user; the kvm guest will name a separate one.
            args.extend([
                "--env",
                "CUBEBOT_TERMINAL_ENABLED=1",
                "--env",
                "CUBEBOT_TOOL_USER=10001:10001",
            ]);
        }
        if self.config.backend == Backend::Kvm {
            // Root with only NET_ADMIN creates the guest's tap in the container network namespace;
            // group 10001 writes the image's workspace volume.
            args.extend([
                "--user",
                "0:10001",
                "--cap-add",
                "NET_ADMIN",
                "--device",
                "/dev/kvm",
                "--device",
                "/dev/net/tun",
                "--env",
                "CUBEBOT_GUEST_WORKSPACE=/var/lib/cubebot/workspace.ext4",
            ]);
        }
        args.extend([
            self.config.image.as_str(),
            "--backend",
            self.config.backend.as_str(),
        ]);
        self.docker(&args).await.map(drop)
    }
    async fn observe(&self, attempt: &Attempt, state: Option<&Value>) -> Result<(), Error> {
        sqlx::query("UPDATE agent_resources SET kubernetes_phase = $2,observed_at = clock_timestamp() WHERE id = $1")
            .bind(attempt.id).bind(phase(state)).execute(&self.pool).await?;
        Ok(())
    }
    /// Stops an attempt and retains its final state. A running agent is drained first, as on
    /// Kubernetes; a container that already exited needs no acknowledgement.
    async fn retire(&self, attempt: &Attempt, mut state: Option<Value>) -> Result<(), Error> {
        if running(state.as_ref()) {
            sqlx::query("INSERT INTO agent_runtime_drains (attempt_id) VALUES ($1) ON CONFLICT (attempt_id) DO NOTHING")
                .bind(attempt.id).execute(&self.pool).await?;
            let acknowledged: bool = sqlx::query_scalar("SELECT acknowledged_at IS NOT NULL FROM agent_runtime_drains WHERE attempt_id = $1")
                .bind(attempt.id).fetch_one(&self.pool).await?;
            if !acknowledged {
                return self.observe(attempt, state.as_ref()).await;
            }
            self.docker(&["container", "stop", &attempt.name]).await?;
            state = self.inspect(&attempt.name).await?;
            if running(state.as_ref()) {
                return Err(Error::TerminationUnproven);
            }
        }
        // A removed container cannot still be running; Docker only removes running ones by force.
        let proof = state.clone().unwrap_or_else(|| json!({"Status":"removed"}));
        sqlx::query("UPDATE agent_resources SET phase = 'stopped',termination_proof = $2,kubernetes_phase = $3,observed_at = clock_timestamp(),updated_at = clock_timestamp() WHERE id = $1")
            .bind(attempt.id).bind(&proof).bind(phase(state.as_ref())).execute(&self.pool).await?;
        if state.is_some() {
            self.docker(&["container", "rm", &attempt.name]).await?;
        }
        Ok(())
    }
}
fn volume(agent: AgentId) -> String {
    format!("cubebot-workspace-{}", agent.0)
}
fn status(state: Option<&Value>) -> &str {
    state
        .and_then(|state| state["Status"].as_str())
        .unwrap_or("removed")
}
fn running(state: Option<&Value>) -> bool {
    matches!(
        status(state),
        "running" | "paused" | "restarting" | "removing"
    )
}
fn phase(state: Option<&Value>) -> &'static str {
    match status(state) {
        "created" => "Pending",
        "running" | "paused" | "restarting" | "removing" => "Running",
        "exited" if state.is_some_and(|state| state["ExitCode"] == 0) => "Succeeded",
        "exited" | "dead" | "removed" => "Failed",
        _ => "Unknown",
    }
}

impl Sandbox for Docker {
    async fn create(&self, agent: AgentId) -> Result<(), Error> {
        let label = format!("cubebot.dev/agent={}", agent.0);
        self.docker(&["volume", "create", "--label", &label, &volume(agent)])
            .await
            .map(drop)
    }
    async fn start(&self, agent: AgentId) -> Result<(), Error> {
        let attempt = match self.current(agent).await? {
            Some(attempt) => attempt,
            None => self.record(agent).await?,
        };
        if attempt.phase == "creating" {
            self.bind(agent, &attempt).await?;
        }
        let state = self.inspect(&attempt.name).await?;
        let (_, generation) = self.intent(agent).await?;
        // A stale epoch or an exited agent is settled first; the next pass starts a new epoch.
        if attempt.target_generation != generation
            || !matches!(
                status(state.as_ref()),
                "created" | "running" | "paused" | "restarting"
            )
        {
            return self.retire(&attempt, state).await;
        }
        if status(state.as_ref()) == "created" {
            self.docker(&["container", "start", &attempt.name]).await?;
        }
        let state = self.inspect(&attempt.name).await?;
        self.observe(&attempt, state.as_ref()).await
    }
    async fn stop(&self, agent: AgentId) -> Result<(), Error> {
        let Some(attempt) = self.current(agent).await? else {
            return Ok(());
        };
        if attempt.phase == "creating" && self.inspect(&attempt.name).await?.is_some() {
            self.bind(agent, &attempt).await?;
        }
        let state = self.inspect(&attempt.name).await?;
        self.retire(&attempt, state).await
    }
    async fn exec_channel(&self, agent: AgentId, command: &[&str]) -> Result<ExecChannel, Error> {
        let attempt = self.current(agent).await?.ok_or(Error::Conflict)?;
        let mut session = Command::new(&self.program)
            .args(["container", "exec", "--interactive", &attempt.name])
            .args(command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(Error::Sandbox)?;
        let stdin = session.stdin.take().ok_or(Error::InvalidConfiguration)?;
        let stdout = session.stdout.take().ok_or(Error::InvalidConfiguration)?;
        Ok(ExecChannel {
            stdin: Box::new(stdin),
            stdout: Box::new(stdout),
            _session: Box::new(session),
        })
    }
    async fn workspace(&self, agent: AgentId) -> Result<Option<String>, Error> {
        let filter = format!("name=^{}$", volume(agent));
        let found = self
            .docker(&["volume", "ls", "--filter", &filter, "--format", "{{.Name}}"])
            .await?;
        Ok((!found.is_empty()).then_some(found))
    }
    async fn status(&self, agent: AgentId) -> Result<Status, Error> {
        let Some(attempt) = self.current(agent).await? else {
            return Ok(Status::Absent);
        };
        let state = self.inspect(&attempt.name).await?;
        Ok(match status(state.as_ref()) {
            "running" | "paused" => Status::Running,
            "created" | "restarting" => Status::Starting,
            "removing" => Status::Stopping,
            _ if attempt.phase == "creating" => Status::Starting,
            _ => Status::Stopped,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn configuration_rejects_values_docker_would_read_as_flags() {
        let valid = serde_json::json!({"image":"cubebot-sandbox:dev","backend":"process","orchestrator_origin":"http://127.0.0.1:8080"});
        assert!(DockerConfig::parse(&valid.to_string()).is_ok());
        let mut kvm = valid.clone();
        kvm["backend"] = serde_json::json!("kvm");
        kvm["orchestrator_origin"] = serde_json::json!("https://cubebot.test");
        assert!(DockerConfig::parse(&kvm.to_string()).is_ok());
        for (key, value) in [
            ("image", serde_json::json!("--privileged")),
            ("image", serde_json::json!("image with space")),
            ("backend", serde_json::json!("none")),
            (
                "orchestrator_origin",
                serde_json::json!("file:///etc/passwd"),
            ),
            (
                "orchestrator_origin",
                serde_json::json!("http://host.docker.internal:8080"),
            ),
            ("unexpected", serde_json::json!(true)),
            ("backend", serde_json::json!("kvm")),
        ] {
            let mut input = valid.clone();
            input[key] = value;
            assert!(DockerConfig::parse(&input.to_string()).is_err(), "{key}");
        }
    }

    #[test]
    fn exited_and_removed_containers_are_never_reported_running() {
        assert_eq!(phase(None), "Failed");
        assert!(!running(None));
        let exited = json!({"Status":"exited","ExitCode":0});
        assert_eq!(phase(Some(&exited)), "Succeeded");
        assert!(running(Some(&json!({"Status":"removing"}))));
    }
}
