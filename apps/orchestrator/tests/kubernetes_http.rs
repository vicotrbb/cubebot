#[path = "support/kubernetes.rs"]
mod fixtures;
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{Method, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::any,
};
use cubebot_orchestrator::agents::kubernetes::{Error, Kubernetes};
use k8s_openapi::api::core::v1::Pod;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
struct ApiState {
    pod: Option<Pod>,
    calls: Vec<(Method, String, Value)>,
    conflict_create: bool,
}

async fn serve(
    State(state): State<Arc<Mutex<ApiState>>>,
    method: Method,
    uri: Uri,
    bytes: Bytes,
) -> Response {
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).unwrap()
    };
    let mut state = state.lock().unwrap();
    state
        .calls
        .push((method.clone(), uri.to_string(), body.clone()));
    if method == Method::POST {
        if state.conflict_create {
            return (StatusCode::CONFLICT,Json(json!({"apiVersion":"v1","kind":"Status","status":"Failure","message":"already exists","reason":"AlreadyExists","code":409}))).into_response();
        }
        let mut created = body;
        created["metadata"]["uid"] = json!("created-uid");
        return Json(created).into_response();
    }
    if let Some(pod) = &state.pod {
        return Json(pod.clone()).into_response();
    }
    (StatusCode::NOT_FOUND,Json(json!({"apiVersion":"v1","kind":"Status","status":"Failure","message":"not found","reason":"NotFound","code":404}))).into_response()
}

async fn server(
    pod: Pod,
) -> (
    Kubernetes,
    Arc<Mutex<ApiState>>,
    tokio::task::JoinHandle<()>,
) {
    let state = Arc::new(Mutex::new(ApiState {
        pod: Some(pod),
        calls: vec![],
        conflict_create: false,
    }));
    let router = Router::new().fallback(any(serve)).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    let client = kube::Client::try_from(kube::Config::new(url.parse().unwrap())).unwrap();
    (
        Kubernetes::new(client, "cubebot-test").unwrap(),
        state,
        task,
    )
}

#[tokio::test]
async fn stop_is_namespaced_uid_guarded_and_never_force_deletes() {
    let identity = fixtures::identity();
    let recorded = fixtures::recorded(identity.clone());
    let (client, state, task) = server(fixtures::pod(&identity)).await;
    client.request_stop(&recorded).await.unwrap();
    let calls = state.lock().unwrap().calls.clone();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[1].0, Method::DELETE);
    assert_eq!(
        calls[1].1,
        "/api/v1/namespaces/cubebot-test/pods/agent-test?"
    );
    assert_eq!(calls[1].2["preconditions"]["uid"], "pod-uid");
    assert_eq!(calls[1].2["preconditions"]["resourceVersion"], "1");
    assert_eq!(calls[1].2["gracePeriodSeconds"], 30);
    task.abort();
}

#[tokio::test]
async fn finalization_requires_observed_termination_and_preserves_foreign_finalizers() {
    let identity = fixtures::identity();
    let recorded = fixtures::recorded(identity.clone());
    let (client, state, task) = server(fixtures::pod(&identity)).await;
    assert!(matches!(
        client.finalize_stopped(&recorded).await,
        Err(Error::TerminationUnproven)
    ));
    fixtures::terminate(state.lock().unwrap().pod.as_mut().unwrap());
    client.finalize_stopped(&recorded).await.unwrap();
    let calls = state.lock().unwrap().calls.clone();
    let patch = &calls.last().unwrap().2;
    assert_eq!(calls.last().unwrap().0, Method::PATCH);
    assert_eq!(
        patch["metadata"]["finalizers"],
        json!(["other-controller.example/retain"])
    );
    assert_eq!(patch["metadata"]["resourceVersion"], "1");
    assert_eq!(patch["metadata"]["uid"], "pod-uid");
    task.abort();
}

#[tokio::test]
async fn missing_or_replaced_pods_never_authorize_stop_or_finalizer_release() {
    let identity = fixtures::identity();
    let recorded = fixtures::recorded(identity.clone());
    let (client, state, task) = server(fixtures::pod(&identity)).await;
    state.lock().unwrap().pod.as_mut().unwrap().metadata.uid = Some("other-uid".into());
    assert!(matches!(
        client.request_stop(&recorded).await,
        Err(Error::OwnershipMismatch)
    ));
    state.lock().unwrap().pod = None;
    assert!(client.observe(&recorded).await.unwrap().is_none());
    assert!(matches!(
        client.finalize_stopped(&recorded).await,
        Err(Error::TerminationUnproven)
    ));
    assert!(
        state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|call| call.0 == Method::GET)
    );
    task.abort();
}

#[tokio::test]
async fn terminal_status_without_a_stop_request_keeps_the_finalizer() {
    let identity = fixtures::identity();
    let recorded = fixtures::recorded(identity.clone());
    let mut pod = fixtures::pod(&identity);
    fixtures::terminate(&mut pod);
    pod.metadata.deletion_timestamp = None;
    let (client, state, task) = server(pod).await;
    assert!(matches!(
        client.finalize_stopped(&recorded).await,
        Err(Error::TerminationUnproven)
    ));
    assert!(
        state
            .lock()
            .unwrap()
            .calls
            .iter()
            .all(|call| call.0 == Method::GET)
    );
    task.abort();
}

#[tokio::test]
async fn creates_only_in_the_selected_namespace_and_never_adopts_conflicting_objects() {
    use k8s_openapi::api::core::v1::PersistentVolumeClaim;
    let identity = fixtures::identity();
    let mut pod = fixtures::pod(&identity);
    pod.metadata.uid = None;
    pod.metadata.resource_version = None;
    let (client, state, task) = server(pod.clone()).await;
    assert_eq!(
        client
            .create_pod(&pod)
            .await
            .unwrap()
            .metadata
            .uid
            .as_deref(),
        Some("created-uid")
    );
    let claim:PersistentVolumeClaim = serde_json::from_value(json!({"metadata":{"name":"workspace-test"},"spec":{"accessModes":["ReadWriteOnce"],"resources":{"requests":{"storage":"1Gi"}}}})).unwrap();
    assert_eq!(
        client
            .create_workspace(&claim)
            .await
            .unwrap()
            .metadata
            .name
            .as_deref(),
        Some("workspace-test")
    );
    state.lock().unwrap().conflict_create = true;
    assert!(
        matches!(client.create_pod(&pod).await,Err(Error::Api(kube::Error::Api(response))) if response.code == 409)
    );
    let calls = state.lock().unwrap().calls.clone();
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|call| call.0 == Method::POST));
    assert_eq!(
        calls[0].1.split('?').next(),
        Some("/api/v1/namespaces/cubebot-test/pods")
    );
    assert_eq!(
        calls[1].1.split('?').next(),
        Some("/api/v1/namespaces/cubebot-test/persistentvolumeclaims")
    );
    task.abort();
}
