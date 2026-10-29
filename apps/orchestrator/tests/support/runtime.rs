use crate::registry_support;
use axum::{
    Json, Router,
    body::Bytes,
    extract::State,
    http::{Method, Uri},
    routing::any,
};
use cubebot_orchestrator::agents::{
    AgentId,
    kubernetes::{AttemptId, Layout, Registry, ResourceObject, ResourceSpec},
    runtime::RuntimeIdentity,
};
use k8s_openapi::api::core::v1::Pod;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

pub struct Api {
    pub pod: Pod,
    pub service_uid: String,
}
async fn api(
    State(state): State<Arc<Mutex<Api>>>,
    method: Method,
    uri: Uri,
    bytes: Bytes,
) -> Json<Value> {
    let state = state.lock().unwrap();
    if method == Method::GET {
        return Json(serde_json::to_value(&state.pod).unwrap());
    }
    assert_eq!(uri.path(), "/apis/authentication.k8s.io/v1/tokenreviews");
    let request: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(request["spec"]["audiences"], json!(["cubebot-runtime"]));
    Json(
        json!({"apiVersion":"authentication.k8s.io/v1","kind":"TokenReview","status":{"authenticated":true,"audiences":["cubebot-runtime"],"user":{"username":"system:serviceaccount:cubebot-test:cubebot-runtime","uid":state.service_uid,"extra":{"authentication.kubernetes.io/pod-name":[state.pod.metadata.name],"authentication.kubernetes.io/pod-uid":[state.pod.metadata.uid]}}}}),
    )
}
pub struct Fixture {
    pub pool: sqlx::PgPool,
    pub auth: RuntimeIdentity,
    pub api: Arc<Mutex<Api>>,
    pub task: tokio::task::JoinHandle<()>,
    pub agent: AgentId,
    pub attempt: AttemptId,
}
pub async fn fixture() -> Fixture {
    let (pool, registry, agent) = registry_support::setup().await;
    bind_anchor(&registry).await;
    let image = "registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let layout = Layout::new(
        image,
        "cubebot-runtime",
        "https://cubebot.test",
        cubebot_protocol::network::NetworkPolicy::new(vec!["10.96.0.10".parse().unwrap()]).unwrap(),
    )
    .unwrap();
    let attempt = registry
        .reserve(
            agent,
            2,
            ResourceSpec::Pod(Box::new(layout.pod("owned-workspace").unwrap())),
        )
        .await
        .unwrap();
    registry
        .record_prepared(attempt.id, &attempt.object)
        .await
        .unwrap();
    registry.begin_create(attempt.id).await.unwrap();
    let ResourceObject::Pod(mut pod) = attempt.object else {
        panic!("pod expected")
    };
    pod.metadata.uid = Some("pod-uid".into());
    pod.metadata.resource_version = Some("1".into());
    pod.status = Some(serde_json::from_value(json!({"phase":"Running"})).unwrap());
    registry
        .bind_object(attempt.id, &ResourceObject::Pod(pod.clone()))
        .await
        .unwrap();
    let state = Arc::new(Mutex::new(Api {
        pod: *pod,
        service_uid: "service-uid".into(),
    }));
    let router = Router::new().fallback(any(api)).with_state(state.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    Fixture {
        auth: RuntimeIdentity::new(
            pool.clone(),
            kube::Config::new(url.parse().unwrap()),
            "cubebot-test",
        )
        .unwrap(),
        pool,
        api: state,
        task,
        agent,
        attempt: attempt.id,
    }
}
async fn bind_anchor(registry: &Registry) {
    let mut anchor = registry.configure("cubebot-test").await.unwrap();
    anchor.metadata.uid = Some("anchor-uid".into());
    anchor.metadata.resource_version = Some("1".into());
    registry.bind_anchor(&anchor).await.unwrap();
}
