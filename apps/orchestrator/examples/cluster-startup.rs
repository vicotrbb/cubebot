//! Actual CSI-backed kvm sandbox startup; the guest never reaches an orchestrator.
#[path = "support/cluster.rs"]
mod cluster;
#[path = "../tests/support/registry.rs"]
mod registry_support;
#[path = "../tests/support/mod.rs"]
mod support;
use cluster::required;
use cubebot_orchestrator::{
    agents::{
        AgentId, Agents, LifecycleAction, LifecycleCommand,
        kubernetes::{
            Attempt, Kubernetes, Layout, Phase, ReconcileResult, Reconciler, Registry,
            ResourceObject,
        },
        runtime::{RuntimeAccess, RuntimeIdentity, RuntimeSession},
    },
    identity::Identity,
};
use cubebot_protocol::network::NetworkPolicy;
use k8s_openapi::api::core::v1::{ContainerState, Event, PersistentVolume, Pod};
use kube::{
    Api, Client,
    api::{DeleteParams, ListParams, LogParams, PostParams, Preconditions},
};
use serde_json::json;
use std::{process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};
/// Fixture failures end the run with their cause.
type Result<T, E = Box<dyn std::error::Error>> = std::result::Result<T, E>;
const MISSING: &str = "an expected fixture field is missing";

/// IdentityError has no std::error::Error impl, so it ends the run as its Debug text.
fn identity<T>(
    result: std::result::Result<T, cubebot_orchestrator::identity::IdentityError>,
) -> Result<T> {
    result.map_err(|error| format!("{error:?}").into())
}

const NAMESPACE: &str = "cubebot-startup";
const MARKER: &str = "/var/lib/cubebot/fixture-retained";
const BYTES: &str = "retained workspace bytes\n";
struct Fixture {
    pool: sqlx::PgPool,
    registry: Registry,
    agent: AgentId,
    api: Kubernetes,
    reconciler: Reconciler,
    client: Client,
    identity: RuntimeIdentity,
}
#[tokio::main]
async fn main() -> Result<()> {
    let fixture = setup().await?;
    let (workspace, first) = provision(&fixture).await?;
    let first_pod = started(&fixture, &first).await?;
    let (projected, session) = verify_identity(&fixture, &first, &first_pod).await?;
    verify_storage(&fixture, &workspace, &first_pod).await?;
    let name = first_pod.metadata.name.as_deref().ok_or(MISSING)?;
    exec(name, &["/usr/bin/tee", MARKER], Some(BYTES)).await?;
    let rejection = verify_rwop(&fixture, &workspace).await?;
    settle(&fixture, &first, &first_pod, 2).await?;
    verify_revoked(&fixture, &projected, &session).await?;
    availability(&fixture, LifecycleAction::Wake, 3).await?;
    let (retained, second) = provision(&fixture).await?;
    let second_pod = started(&fixture, &second).await?;
    let (_, second_session) = verify_identity(&fixture, &second, &second_pod).await?;
    assert!(second_session.epoch > session.epoch);
    assert_eq!(workspace.uid, retained.uid);
    assert_ne!(first_pod.metadata.uid, second_pod.metadata.uid);
    let name = second_pod.metadata.name.as_deref().ok_or(MISSING)?;
    assert_eq!(exec(name, &["/usr/bin/cat", MARKER], None).await?, BYTES);
    settle(&fixture, &second, &second_pod, 4).await?;
    println!(
        "{}",
        json!({"workspace_uid":retained.uid,"first_pod_uid":first_pod.metadata.uid,
        "second_pod_uid":second_pod.metadata.uid,"sandbox_started":true,"sandbox_backend":"kvm",
        "rwop_scheduler_rejected_second_mount":true,"rwop_rejection":rejection,"workspace_bytes_survived":true,
        "working_agent":false,"runtime_authenticated":false,
        "actual_projected_token_exchange":true,"different_process_rejected":true,
        "removed_pod_credentials_rejected":true,"replacement_epoch_increased":true,
        "operator_stop_required":true,"scope":"CSI/layout startup and clean termination only; G1-G3 remain open"})
    );
    fixture.pool.close().await;
    Ok(())
}
async fn setup() -> Result<Fixture> {
    let config = cluster::configuration().await?;
    cluster::verify_namespace(&config, NAMESPACE).await?;
    let (pool, registry, agent) = registry_support::setup().await;
    let api = Kubernetes::new(config.clone(), NAMESPACE)?;
    let client = Client::try_from(config.clone())?;
    let identity = RuntimeIdentity::new(pool.clone(), config.clone(), NAMESPACE)?;
    let reconciler = Reconciler::new(pool.clone(), config, NAMESPACE, layout()?)?;
    Ok(Fixture {
        pool,
        registry,
        agent,
        api,
        reconciler,
        client,
        identity,
    })
}
async fn verify_identity(
    f: &Fixture,
    attempt: &Attempt,
    pod: &Pod,
) -> Result<(String, RuntimeSession)> {
    let projected = exec(
        pod.metadata.name.as_deref().ok_or(MISSING)?,
        &["/usr/bin/cat", "/run/cubebot/identity/token"],
        None,
    )
    .await?;
    let instance = uuid::Uuid::new_v4();
    let session = f.identity.bootstrap(&projected, instance).await?;
    assert_eq!(session.agent, f.agent);
    assert_eq!(session.attempt, attempt.id);
    assert_eq!(session.epoch, attempt.epoch);
    let actor = f
        .identity
        .authenticate(&session.token, RuntimeAccess::Active)
        .await?;
    assert_eq!(actor.attempt, attempt.id);
    assert!(matches!(
        f.identity.bootstrap(&projected, uuid::Uuid::new_v4()).await,
        Err(cubebot_orchestrator::agents::kubernetes::Error::Conflict)
    ));
    Ok((projected, session))
}
async fn verify_revoked(f: &Fixture, projected: &str, session: &RuntimeSession) -> Result<()> {
    use cubebot_orchestrator::agents::kubernetes::Error;
    assert!(matches!(
        f.identity
            .authenticate(&session.token, RuntimeAccess::Active)
            .await,
        Err(Error::Unauthorized)
    ));
    assert!(matches!(
        f.identity.bootstrap(projected, uuid::Uuid::new_v4()).await,
        Err(Error::Unauthorized)
    ));
    Ok(())
}
fn layout() -> Result<Layout> {
    let network = NetworkPolicy::new(vec![required("CUBEBOT_CLUSTER_DNS")?.parse()?])?;
    Ok(Layout::new(
        &required("CUBEBOT_SANDBOX_IMAGE")?,
        "cubebot-runtime",
        "https://127.0.0.1:1",
        network,
    )?
    .with_terminal(true))
}
async fn provision(f: &Fixture) -> Result<(Attempt, Attempt)> {
    let ReconcileResult::Provisioned { workspace, pod } = f.reconciler.reconcile(f.agent).await?
    else {
        panic!("expected actual provisioning");
    };
    Ok((
        f.registry.attempt(workspace).await?,
        f.registry.attempt(pod).await?,
    ))
}
fn state<'a>(pod: &'a Pod, name: &str) -> Option<&'a ContainerState> {
    pod.status
        .as_ref()?
        .container_statuses
        .as_ref()?
        .iter()
        .find(|item| item.name == name)?
        .state
        .as_ref()
}
fn startup_finished(pod: &Pod) -> bool {
    state(pod, "sandbox").is_some_and(|state| state.running.is_some())
}
async fn started(f: &Fixture, attempt: &Attempt) -> Result<Pod> {
    for _ in 0..120 {
        let Some(ResourceObject::Pod(pod)) = f.api.fetch_candidate(&attempt.object).await? else {
            panic!("owned pod missing");
        };
        save("observed-pod.json", &pod)?;
        assert_eq!(pod.metadata.uid, attempt.uid);
        if startup_finished(&pod) {
            let logs = Api::<Pod>::namespaced(f.client.clone(), NAMESPACE)
                .logs(
                    pod.metadata.name.as_deref().ok_or(MISSING)?,
                    &LogParams {
                        container: Some("sandbox".into()),
                        tail_lines: Some(10),
                        limit_bytes: Some(1024),
                        ..Default::default()
                    },
                )
                .await?;
            assert!(!logs.contains("Sandbox failed"));
            save("sandbox-log.json", &logs)?;
            return Ok(*pod);
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    panic!("CSI-backed sandbox layout did not start; inspect observed-pod.json");
}
fn save(name: &str, value: &impl serde::Serialize) -> Result<()> {
    let root = std::path::PathBuf::from(required("CUBEBOT_CLUSTER_ARTIFACTS")?);
    std::fs::write(root.join(name), serde_json::to_vec_pretty(value)?)?;
    Ok(())
}
async fn verify_storage(f: &Fixture, workspace: &Attempt, pod: &Pod) -> Result<()> {
    let Some(ResourceObject::Workspace(claim)) = f.api.fetch_candidate(&workspace.object).await?
    else {
        panic!("workspace missing");
    };
    save("observed-workspace.json", &claim)?;
    assert_eq!(
        claim.status.as_ref().ok_or(MISSING)?.phase.as_deref(),
        Some("Bound")
    );
    assert_eq!(
        claim.spec.as_ref().ok_or(MISSING)?.access_modes.as_deref(),
        Some(["ReadWriteOncePod".to_owned()].as_slice())
    );
    assert_eq!(
        claim
            .spec
            .as_ref()
            .ok_or(MISSING)?
            .storage_class_name
            .as_deref(),
        Some("cubebot-fixture-csi")
    );
    let volume = Api::<PersistentVolume>::all(f.client.clone())
        .get(
            claim
                .spec
                .as_ref()
                .ok_or(MISSING)?
                .volume_name
                .as_deref()
                .ok_or(MISSING)?,
        )
        .await?;
    assert_eq!(
        volume
            .spec
            .as_ref()
            .ok_or(MISSING)?
            .csi
            .as_ref()
            .ok_or(MISSING)?
            .driver,
        "hostpath.csi.k8s.io"
    );
    let name = pod.metadata.name.as_deref().ok_or(MISSING)?;
    // The journal lives inside the guest's workspace disk, which only the root sandbox opens.
    let owner = exec(
        name,
        &[
            "/usr/bin/stat",
            "-c",
            "%u",
            "/var/lib/cubebot/workspace.ext4",
        ],
        None,
    )
    .await?;
    assert_eq!(owner.trim(), "0");
    Ok(())
}
async fn availability(f: &Fixture, action: LifecycleAction, revision: i64) -> Result<()> {
    let identity = Identity::new(f.pool.clone());
    let login = self::identity(identity.login("admin", "administrator password").await)?;
    let actor = self::identity(identity.authenticate(&login.token).await)?;
    let changed = Agents::new(f.pool.clone())
        .lifecycle(
            &actor,
            f.agent,
            &LifecycleCommand {
                request_id: uuid::Uuid::new_v4(),
                expected_revision: revision,
                action,
            },
        )
        .await;
    self::identity(changed)?;
    Ok(())
}
async fn settle(f: &Fixture, attempt: &Attempt, pod: &Pod, revision: i64) -> Result<()> {
    availability(f, LifecycleAction::Sleep, revision).await?;
    assert!(matches!(
        f.reconciler.reconcile(f.agent).await?,
        ReconcileResult::AwaitingDrain { .. }
    ));
    operator_stop(f, attempt, pod).await?;
    for _ in 0..60 {
        f.reconciler.reconcile(f.agent).await?;
        if f.registry.attempt(attempt.id).await?.phase == Phase::Stopped
            && f.api.fetch_candidate(&attempt.object).await?.is_none()
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    panic!("operator-stopped pod did not settle");
}
/// The unavailable control endpoint cannot acknowledge drain. This is an explicit
/// fixture-operator stop, never a fabricated runtime acknowledgement.
async fn operator_stop(f: &Fixture, attempt: &Attempt, pod: &Pod) -> Result<()> {
    let Some(ResourceObject::Pod(current)) = f.api.fetch_candidate(&attempt.object).await? else {
        panic!("pod missing before operator stop");
    };
    assert_eq!(current.metadata.uid, pod.metadata.uid);
    Api::<Pod>::namespaced(f.client.clone(), NAMESPACE)
        .delete(
            current.metadata.name.as_deref().ok_or(MISSING)?,
            &DeleteParams {
                grace_period_seconds: Some(30),
                preconditions: Some(Preconditions {
                    uid: current.metadata.uid.clone(),
                    resource_version: current.metadata.resource_version.clone(),
                }),
                ..Default::default()
            },
        )
        .await?;
    Ok(())
}
async fn exec(pod: &str, args: &[&str], input: Option<&str>) -> Result<String> {
    tokio::time::timeout(Duration::from_secs(20), async {
        let mut child = Command::new("kubectl")
            .args([
                "--kubeconfig",
                &required("CUBEBOT_CLUSTER_KUBECONFIG")?,
                "--namespace",
                NAMESPACE,
                "exec",
                "--stdin",
                pod,
                "--container",
                "sandbox",
                "--",
            ])
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        if let Some(input) = input {
            child
                .stdin
                .as_mut()
                .ok_or(MISSING)?
                .write_all(input.as_bytes())
                .await?;
        }
        drop(child.stdin.take());
        let output = child.wait_with_output().await?;
        assert!(
            output.status.success(),
            "fixture exec failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        Ok(String::from_utf8(output.stdout)?)
    })
    .await?
}
async fn verify_rwop(f: &Fixture, workspace: &Attempt) -> Result<String> {
    let pods = Api::<Pod>::namespaced(f.client.clone(), NAMESPACE);
    let claim = workspace.object.metadata().name.as_deref().ok_or(MISSING)?;
    let contender: Pod =
        serde_json::from_value(json!({"metadata":{"name":"rwop-contender"},"spec":{
        "restartPolicy":"Never","automountServiceAccountToken":false,
        "containers":[{"name":"contender","image":required("CUBEBOT_SANDBOX_IMAGE")?,
            "imagePullPolicy":"IfNotPresent","command":["/usr/bin/true"],
            "volumeMounts":[{"name":"workspace","mountPath":"/workspace"}]}],
        "volumes":[{"name":"workspace","persistentVolumeClaim":{"claimName":claim}}]}}))?;
    let created = pods.create(&PostParams::default(), &contender).await?;
    let uid = created.metadata.uid.ok_or(MISSING)?;
    let rejection = wait_rwop(f, &uid).await?;
    let observed = pods.get("rwop-contender").await?;
    pods.delete(
        "rwop-contender",
        &DeleteParams {
            grace_period_seconds: Some(0),
            preconditions: Some(Preconditions {
                uid: Some(uid),
                resource_version: observed.metadata.resource_version,
            }),
            ..Default::default()
        },
    )
    .await?;
    Ok(rejection)
}
async fn wait_rwop(f: &Fixture, uid: &str) -> Result<String> {
    let events = Api::<Event>::namespaced(f.client.clone(), NAMESPACE);
    for _ in 0..45 {
        let found = events
            .list(&ListParams::default().fields(&format!("involvedObject.uid={uid}")))
            .await?;
        if let Some(event) = found.items.iter().find(|event| {
            event.reason.as_deref() == Some("FailedScheduling")
                && event
                    .message
                    .as_ref()
                    .is_some_and(|message| message.contains("ReadWriteOncePod"))
        }) {
            return event
                .message
                .clone()
                .ok_or_else(|| "rejection message missing".into());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    panic!("missing actual RWOP scheduler rejection");
}
