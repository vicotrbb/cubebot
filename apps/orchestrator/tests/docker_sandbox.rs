use crate::registry_support;
use cubebot_orchestrator::agents::{
    AgentId,
    kubernetes::Error,
    runtime::{self, RuntimeAccess, RuntimeIdentity},
    sandbox::{Docker, DockerConfig, Sandbox, Status, drive},
};
use std::{os::unix::fs::PermissionsExt, path::PathBuf};
use uuid::Uuid;

// Keeps each container's status in a file named after it; records create arguments.
const FAKE: &str = r#"#!/bin/sh
state="$(dirname "$0")/state"; mkdir -p "$state"
name() { while [ "$#" -gt 1 ]; do [ "$1" = --name ] && { echo "$2"; return; }; shift; done; }
case "$1 $2" in
"volume create") eval "echo \${$#}" ;;
"volume ls") echo "${4#name=^}" | tr -d '$' ;;
"container create") n=$(name "$@"); printf '%s\n' "$@" > "$state/$n.args"; printf '%s' "$CUBEBOT_BOOTSTRAP_TOKEN" > "$state/$n.token"; echo created > "$state/$n"; echo "id-$n" ;;
"container inspect")
  n=$5; [ -f "$state/$n" ] || { echo "Error: No such container: $n" >&2; exit 1; }
  if [ "$4" = '{{.Id}}' ]; then echo "id-$n"; else printf '{"Status":"%s","ExitCode":%s}\n' "$(cat "$state/$n")" "${EXIT:-0}"; fi ;;
"container start") echo running > "$state/$3" ;;
"container stop") echo exited > "$state/$3" ;;
"container rm") rm "$state/$3" "$state/$3.args" "$state/$3.token" ;;
*) echo "unexpected: $*" >&2; exit 1 ;;
esac
"#;

struct Fixture {
    pool: sqlx::PgPool,
    agent: AgentId,
    docker: Docker,
    root: PathBuf,
}
async fn fixture() -> Fixture {
    let (pool, _, agent) = registry_support::setup().await;
    let root = std::env::temp_dir().join(format!("cubebot-docker-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let program = root.join("docker");
    std::fs::write(&program, FAKE).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = DockerConfig::parse(
        r#"{"image":"cubebot-sandbox:test","orchestrator_origin":"https://cubebot.test"}"#,
    )
    .unwrap();
    let docker = Docker::new(pool.clone(), config).with_program(program.to_str().unwrap());
    Fixture {
        pool,
        agent,
        docker,
        root,
    }
}
impl Fixture {
    fn container(&self, epoch: i64) -> String {
        format!("cubebot-agent-{}-{epoch}", self.agent.0)
    }
    /// The token reaches the CLI through its environment, never its arguments.
    fn token(&self, epoch: i64) -> String {
        let state = self.root.join("state");
        let read = |suffix: &str| {
            std::fs::read_to_string(state.join(format!("{}.{suffix}", self.container(epoch))))
                .unwrap()
        };
        let (args, token) = (read("args"), read("token"));
        assert!(token.starts_with("cb_sandbox_") && !args.contains("cb_sandbox_"));
        assert!(args.lines().any(|line| line == "CUBEBOT_BOOTSTRAP_TOKEN"));
        assert!(args.contains("\n/dev/kvm\n") && args.ends_with("\ncubebot-sandbox:test\nrun\n"));
        token
    }
    fn set(&self, epoch: i64, status: &str) {
        std::fs::write(self.root.join("state").join(self.container(epoch)), status).unwrap();
    }
    async fn phases(&self) -> Vec<(i64, String)> {
        sqlx::query_as("SELECT epoch,phase FROM agent_resources WHERE agent_id = $1 AND kind = 'pod' ORDER BY epoch")
            .bind(self.agent).fetch_all(&self.pool).await.unwrap()
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn container_token_authenticates_only_its_own_running_epoch() {
    let f = fixture().await;
    drive(&f.docker, &f.pool, f.agent).await.unwrap();
    assert_eq!(f.docker.status(f.agent).await.unwrap(), Status::Running);
    assert_eq!(
        f.docker.workspace(f.agent).await.unwrap().unwrap(),
        format!("cubebot-workspace-{}", f.agent.0)
    );
    let identity = RuntimeIdentity::docker(f.pool.clone());
    let first = f.token(1);
    let session = identity.bootstrap(&first, Uuid::new_v4()).await.unwrap();
    assert_eq!(session.agent, f.agent);
    assert_eq!(session.epoch, 1);
    identity
        .authenticate(&session.token, RuntimeAccess::Active)
        .await
        .unwrap();
    let forged = format!("cb_sandbox_{}", "0".repeat(64));
    assert!(matches!(
        identity.bootstrap(&forged, Uuid::new_v4()).await,
        Err(Error::Unauthorized)
    ));
    // A crashed container is retired with its final state before a new epoch starts.
    f.set(1, "exited");
    drive(&f.docker, &f.pool, f.agent).await.unwrap();
    assert!(
        identity
            .authenticate(&session.token, RuntimeAccess::Recovery)
            .await
            .is_err()
    );
    drive(&f.docker, &f.pool, f.agent).await.unwrap();
    assert_eq!(
        f.phases().await,
        vec![(1, "stopped".into()), (2, "bound".into())]
    );
    assert!(matches!(
        identity.bootstrap(&first, Uuid::new_v4()).await,
        Err(Error::Unauthorized)
    ));
    identity
        .bootstrap(&f.token(2), Uuid::new_v4())
        .await
        .unwrap();
    std::fs::remove_dir_all(&f.root).unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn sleep_and_restart_wait_for_drain_before_stopping_a_running_agent() {
    let f = fixture().await;
    drive(&f.docker, &f.pool, f.agent).await.unwrap();
    // Restart: lifecycle bumps the generation and requests a drain of the old epoch.
    sqlx::query("UPDATE agents SET restart_generation = restart_generation + 1 WHERE id = $1")
        .bind(f.agent)
        .execute(&f.pool)
        .await
        .unwrap();
    drive(&f.docker, &f.pool, f.agent).await.unwrap();
    assert_eq!(f.phases().await, vec![(1, "bound".into())]);
    assert_eq!(f.docker.status(f.agent).await.unwrap(), Status::Running);
    sqlx::query("UPDATE agent_runtime_drains SET acknowledged_at = clock_timestamp(),acknowledged_session_version = 1 WHERE attempt_id = (SELECT id FROM agent_resources WHERE agent_id = $1 AND epoch = 1)")
        .bind(f.agent).execute(&f.pool).await.unwrap();
    drive(&f.docker, &f.pool, f.agent).await.unwrap();
    drive(&f.docker, &f.pool, f.agent).await.unwrap();
    assert_eq!(
        f.phases().await,
        vec![(1, "stopped".into()), (2, "bound".into())]
    );
    // Sleep keeps the workspace and leaves no running container once drained.
    sqlx::query("UPDATE agents SET desired_state = 'sleeping' WHERE id = $1")
        .bind(f.agent)
        .execute(&f.pool)
        .await
        .unwrap();
    drive(&f.docker, &f.pool, f.agent).await.unwrap();
    assert_eq!(f.docker.status(f.agent).await.unwrap(), Status::Running);
    sqlx::query("UPDATE agent_runtime_drains SET acknowledged_at = clock_timestamp(),acknowledged_session_version = 1 WHERE attempt_id = (SELECT id FROM agent_resources WHERE agent_id = $1 AND epoch = 2)")
        .bind(f.agent).execute(&f.pool).await.unwrap();
    drive(&f.docker, &f.pool, f.agent).await.unwrap();
    assert_eq!(f.docker.status(f.agent).await.unwrap(), Status::Absent);
    assert!(f.docker.workspace(f.agent).await.unwrap().is_some());
    std::fs::remove_dir_all(&f.root).unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_revoked_credential_authorizes_nothing_and_cannot_be_reissued() {
    let f = fixture().await;
    drive(&f.docker, &f.pool, f.agent).await.unwrap();
    let identity = RuntimeIdentity::docker(f.pool.clone());
    let instance = Uuid::new_v4();
    let session = identity.bootstrap(&f.token(1), instance).await.unwrap();
    assert_eq!(runtime::revoke(&f.pool, f.agent).await.unwrap(), 1);
    assert!(matches!(
        identity
            .authenticate(&session.token, RuntimeAccess::Recovery)
            .await,
        Err(Error::Unauthorized)
    ));
    // The same container cannot exchange its bootstrap token for a fresh credential.
    assert!(identity.bootstrap(&f.token(1), instance).await.is_err());
    assert_eq!(runtime::revoke(&f.pool, f.agent).await.unwrap(), 0);
    std::fs::remove_dir_all(&f.root).unwrap();
}
