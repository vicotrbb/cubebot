use crate::support;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
};
use cubebot_orchestrator::{
    agents::kubernetes::Kubernetes,
    operations::ownership::{Owner, PodIdentity},
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
use uuid::Uuid;
type Pods = Arc<Mutex<BTreeMap<String, Value>>>;
async fn read(State(pods): State<Pods>, Path(name): Path<String>) -> Response {
    match pods.lock().unwrap().get(&name).cloned() {
        Some(pod) => Json(pod).into_response(),
        None => (StatusCode::NOT_FOUND, Json(json!({"apiVersion":"v1","kind":"Status","status":"Failure","reason":"NotFound","message":"missing fixture pod","code":404}))).into_response(),
    }
}

fn pod(name: &str, uid: &str, container: &str) -> Value {
    json!({"apiVersion":"v1","kind":"Pod","metadata":{"name":name,"namespace":"cubebot-test","uid":uid},"spec":{"restartPolicy":"Always","serviceAccountName":"cubebot-orchestrator","containers":[{"name":"orchestrator","image":"test"}]},"status":{"phase":"Running","containerStatuses":[{"name":"orchestrator","image":"test","imageID":"test","ready":true,"restartCount":0,"containerID":container,"state":{"running":{"startedAt":"2026-10-26T12:00:00Z"}}}]}})
}
struct Fixture {
    pool: sqlx::PgPool,
    api: Kubernetes,
    pods: Pods,
    task: tokio::task::JoinHandle<()>,
}
async fn fixture() -> Fixture {
    let pool = support::database().await;
    let pods = Arc::new(Mutex::new(BTreeMap::from([(
        "orchestrator-a".into(),
        pod("orchestrator-a", "pod-a", "containerd://a"),
    )])));
    let router = Router::new()
        .route("/api/v1/namespaces/cubebot-test/pods/{name}", get(read))
        .with_state(pods.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let api = Kubernetes::new(kube::Config::new(origin.parse().unwrap()), "cubebot-test").unwrap();
    Fixture {
        pool,
        api,
        pods,
        task,
    }
}
fn identity(name: &str, uid: &str) -> PodIdentity {
    PodIdentity {
        namespace: "cubebot-test".into(),
        name: name.into(),
        uid: uid.into(),
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn one_live_owner_excludes_competitors_and_clean_release_allows_restart() {
    let f = fixture().await;
    let mut owner = Owner::acquire(&f.pool, &f.api, identity("orchestrator-a", "pod-a"))
        .await
        .unwrap();
    owner.check().await.unwrap();
    assert!(
        Owner::acquire(&f.pool, &f.api, identity("orchestrator-a", "pod-a"))
            .await
            .is_err()
    );
    let first = owner.instance();
    owner.release().await.unwrap();
    let next = Owner::acquire(&f.pool, &f.api, identity("orchestrator-a", "pod-a"))
        .await
        .unwrap();
    assert_ne!(first, next.instance());
    assert_eq!(next.generation(), 2);
    next.release().await.unwrap();
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn connection_loss_and_missing_pod_never_prove_the_old_owner_stopped() {
    let f = fixture().await;
    let owner = Owner::acquire(&f.pool, &f.api, identity("orchestrator-a", "pod-a"))
        .await
        .unwrap();
    let instance = owner.instance();
    drop(owner);
    wait_unlocked(&f.pool).await;
    f.pods.lock().unwrap().insert(
        "orchestrator-b".into(),
        pod("orchestrator-b", "pod-b", "containerd://b"),
    );
    assert!(matches!(
        Owner::acquire(&f.pool, &f.api, identity("orchestrator-b", "pod-b")).await,
        Err(cubebot_orchestrator::agents::kubernetes::Error::TerminationUnproven)
    ));
    wait_unlocked(&f.pool).await;
    f.pods.lock().unwrap().remove("orchestrator-a");
    assert!(matches!(
        Owner::acquire(&f.pool, &f.api, identity("orchestrator-b", "pod-b")).await,
        Err(cubebot_orchestrator::agents::kubernetes::Error::TerminationUnproven)
    ));
    wait_unlocked(&f.pool).await;
    let retained: Uuid =
        sqlx::query_scalar("SELECT instance_id FROM orchestrator_owner WHERE singleton")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(retained, instance);
    f.task.abort();
}

async fn wait_unlocked(pool: &sqlx::PgPool) {
    for _ in 0..100 {
        let unlocked: bool = sqlx::query_scalar("SELECT NOT EXISTS(SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND database = (SELECT oid FROM pg_database WHERE datname = current_database()))")
            .fetch_one(pool).await.unwrap();
        if unlocked {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("owned advisory session did not close");
}
fn terminated(container: &str) -> Value {
    json!({"exitCode":1,"reason":"Error","containerID":container,"startedAt":"2026-10-26T11:59:00Z","finishedAt":"2026-10-26T11:59:59Z","message":"private-termination-message"})
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn same_pod_container_restart_requires_matching_prior_termination_and_redacts_proof() {
    let f = fixture().await;
    let owner = Owner::acquire(&f.pool, &f.api, identity("orchestrator-a", "pod-a"))
        .await
        .unwrap();
    let instance = owner.instance();
    drop(owner);
    wait_unlocked(&f.pool).await;
    let mut restarted = pod("orchestrator-a", "pod-a", "containerd://b");
    restarted["status"]["containerStatuses"][0]["lastState"] =
        json!({"terminated":terminated("containerd://a")});
    f.pods
        .lock()
        .unwrap()
        .insert("orchestrator-a".into(), restarted);
    let mut next = Owner::acquire(&f.pool, &f.api, identity("orchestrator-a", "pod-a"))
        .await
        .unwrap();
    assert_eq!(next.generation(), 2);
    next.check().await.unwrap();
    let proof: String = sqlx::query_scalar(
        "SELECT previous_termination_proof::text FROM orchestrator_owner WHERE singleton",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert!(proof.contains(&instance.to_string()));
    assert!(proof.contains("containerd://a"));
    assert!(!proof.contains("private-termination-message"));
    next.release().await.unwrap();
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn different_pod_takeover_rejects_node_lost_until_exact_terminal_evidence_exists() {
    use cubebot_orchestrator::agents::kubernetes::Error;
    let f = fixture().await;
    let owner = Owner::acquire(&f.pool, &f.api, identity("orchestrator-a", "pod-a"))
        .await
        .unwrap();
    drop(owner);
    wait_unlocked(&f.pool).await;
    let mut stopped = pod("orchestrator-a", "pod-a", "containerd://a");
    stopped["metadata"]["deletionTimestamp"] = json!("2026-10-26T12:00:00Z");
    stopped["status"]["phase"] = json!("Failed");
    stopped["status"]["reason"] = json!("NodeLost");
    stopped["status"]["containerStatuses"][0]["state"] =
        json!({"terminated":terminated("containerd://a")});
    f.pods
        .lock()
        .unwrap()
        .insert("orchestrator-a".into(), stopped.clone());
    f.pods.lock().unwrap().insert(
        "orchestrator-b".into(),
        pod("orchestrator-b", "pod-b", "containerd://b"),
    );
    assert!(matches!(
        Owner::acquire(&f.pool, &f.api, identity("orchestrator-b", "pod-b")).await,
        Err(Error::TerminationUnproven)
    ));
    wait_unlocked(&f.pool).await;
    stopped["status"].as_object_mut().unwrap().remove("reason");
    f.pods
        .lock()
        .unwrap()
        .insert("orchestrator-a".into(), stopped);
    let next = Owner::acquire(&f.pool, &f.api, identity("orchestrator-b", "pod-b"))
        .await
        .unwrap();
    assert_eq!(next.generation(), 2);
    next.release().await.unwrap();
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn reused_pod_name_and_unmatched_container_history_cannot_erase_the_claim() {
    use cubebot_orchestrator::agents::kubernetes::Error;
    let f = fixture().await;
    let owner = Owner::acquire(&f.pool, &f.api, identity("orchestrator-a", "pod-a"))
        .await
        .unwrap();
    drop(owner);
    wait_unlocked(&f.pool).await;
    f.pods.lock().unwrap().insert(
        "orchestrator-a".into(),
        pod("orchestrator-a", "pod-replaced", "containerd://b"),
    );
    assert!(matches!(
        Owner::acquire(&f.pool, &f.api, identity("orchestrator-a", "pod-replaced")).await,
        Err(Error::OwnershipMismatch)
    ));
    wait_unlocked(&f.pool).await;
    let mut restarted = pod("orchestrator-a", "pod-a", "containerd://b");
    restarted["status"]["containerStatuses"][0]["lastState"] =
        json!({"terminated":terminated("containerd://unrecorded")});
    f.pods
        .lock()
        .unwrap()
        .insert("orchestrator-a".into(), restarted);
    assert!(matches!(
        Owner::acquire(&f.pool, &f.api, identity("orchestrator-a", "pod-a")).await,
        Err(Error::TerminationUnproven)
    ));
    let generation: i64 =
        sqlx::query_scalar("SELECT generation FROM orchestrator_owner WHERE singleton")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(generation, 1);
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn lost_ownership_connection_poisons_the_guard_and_never_cleanly_releases_claim() {
    let f = fixture().await;
    let mut owner = Owner::acquire(&f.pool, &f.api, identity("orchestrator-a", "pod-a"))
        .await
        .unwrap();
    let pid: i32 = sqlx::query_scalar("SELECT pid FROM pg_locks WHERE locktype = 'advisory' AND database = (SELECT oid FROM pg_database WHERE datname = current_database())")
        .fetch_one(&f.pool).await.unwrap();
    sqlx::query("SELECT pg_terminate_backend($1)")
        .bind(pid)
        .execute(&f.pool)
        .await
        .unwrap();
    assert!(owner.check().await.is_err());
    assert!(owner.check().await.is_err());
    assert!(owner.release().await.is_err());
    wait_unlocked(&f.pool).await;
    let released: bool = sqlx::query_scalar(
        "SELECT released_at IS NOT NULL FROM orchestrator_owner WHERE singleton",
    )
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert!(!released);
    f.task.abort();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn invalid_current_platform_identity_never_creates_an_owner_claim() {
    use cubebot_orchestrator::agents::kubernetes::Error;
    let f = fixture().await;
    for case in [
        "namespace",
        "uid",
        "default-account",
        "extra-container",
        "ephemeral",
        "missing-start",
    ] {
        let invalid = invalid_current(case);
        f.pods
            .lock()
            .unwrap()
            .insert("orchestrator-a".into(), invalid);
        assert!(matches!(
            Owner::acquire(&f.pool, &f.api, identity("orchestrator-a", "pod-a")).await,
            Err(Error::OwnershipMismatch)
        ));
        wait_unlocked(&f.pool).await;
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM orchestrator_owner")
            .fetch_one(&f.pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
    f.task.abort();
}
fn invalid_current(case: &str) -> Value {
    let mut value = pod("orchestrator-a", "pod-a", "containerd://a");
    match case {
        "namespace" => value["metadata"]["namespace"] = json!("foreign"),
        "uid" => value["metadata"]["uid"] = json!("foreign-uid"),
        "default-account" => value["spec"]["serviceAccountName"] = json!("default"),
        "extra-container" => value["spec"]["containers"]
            .as_array_mut()
            .unwrap()
            .push(json!({"name":"sidecar","image":"test"})),
        "ephemeral" => {
            value["spec"]["ephemeralContainers"] = json!([{"name":"debugger","image":"test"}])
        }
        "missing-start" => value["status"]["containerStatuses"][0]["state"]["running"] = json!({}),
        _ => panic!("unknown identity case"),
    }
    value
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn database_only_owner_excludes_activation_and_retains_unclean_claim() {
    let f = fixture().await;
    let owner = Owner::database_only(&f.pool).await.unwrap();
    assert!(
        Owner::acquire(&f.pool, &f.api, identity("orchestrator-a", "pod-a"))
            .await
            .is_err()
    );
    assert!(Owner::database_only(&f.pool).await.is_err());
    owner.release().await.unwrap();
    let mut owner = Owner::database_only(&f.pool).await.unwrap();
    owner.check().await.unwrap();
    drop(owner);
    wait_unlocked(&f.pool).await;
    assert!(Owner::database_only(&f.pool).await.is_err());
    assert!(matches!(
        Owner::acquire(&f.pool, &f.api, identity("orchestrator-a", "pod-a")).await,
        Err(cubebot_orchestrator::agents::kubernetes::Error::TerminationUnproven)
    ));
    f.task.abort();
}
