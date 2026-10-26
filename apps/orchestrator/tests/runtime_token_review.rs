use axum::{Json, Router, extract::State, http::StatusCode, routing::post};
use cubebot_orchestrator::agents::kubernetes::Kubernetes;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};

fn status() -> Value {
    json!({"authenticated":true,"audiences":["cubebot-runtime"],"user":{"username":"system:serviceaccount:cubebot-test:cubebot-runtime","uid":"service-account-uid","extra":{"authentication.kubernetes.io/pod-name":["agent-pod"],"authentication.kubernetes.io/pod-uid":["pod-uid"]}}})
}
#[derive(Clone)]
struct StateData {
    status: Value,
    requests: Arc<Mutex<Vec<Value>>>,
}
async fn review(
    State(state): State<StateData>,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    state.requests.lock().unwrap().push(body);
    (
        StatusCode::CREATED,
        Json(
            json!({"apiVersion":"authentication.k8s.io/v1","kind":"TokenReview","status":state.status}),
        ),
    )
}
async fn server(
    status: Value,
) -> (
    Kubernetes,
    Arc<Mutex<Vec<Value>>>,
    tokio::task::JoinHandle<()>,
) {
    let requests = Arc::new(Mutex::new(vec![]));
    let router = Router::new()
        .route("/apis/authentication.k8s.io/v1/tokenreviews", post(review))
        .with_state(StateData {
            status,
            requests: requests.clone(),
        });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (
        Kubernetes::new(kube::Config::new(url.parse().unwrap()), "cubebot-test").unwrap(),
        requests,
        task,
    )
}
#[tokio::test]
async fn reviewed_identity_requires_the_runtime_audience_and_pod_binding() {
    let (api, requests, task) = server(status()).await;
    let identity = api
        .review_runtime_token("controlled-projected-token")
        .await
        .unwrap();
    let identity = serde_json::to_value(identity).unwrap();
    assert_eq!(identity["pod_uid"], "pod-uid");
    assert_eq!(identity["namespace"], "cubebot-test");
    assert_eq!(
        requests.lock().unwrap()[0]["spec"]["audiences"],
        json!(["cubebot-runtime"])
    );
    assert_eq!(
        requests.lock().unwrap()[0]["spec"]["token"],
        "controlled-projected-token"
    );
    task.abort();
}
#[tokio::test]
async fn authentication_failures_and_unbound_or_ambiguous_identities_are_rejected() {
    for (path, value) in [
        ("/authenticated", json!(false)),
        ("/audiences", json!(["kubernetes-api"])),
        (
            "/user/username",
            json!("system:serviceaccount:foreign:cubebot-runtime"),
        ),
        (
            "/user/extra/authentication.kubernetes.io~1pod-uid",
            json!([]),
        ),
        (
            "/user/extra/authentication.kubernetes.io~1pod-name",
            json!(["one", "two"]),
        ),
        ("/user/uid", json!("")),
        ("/error", json!("invalid credential")),
    ] {
        let mut response = status();
        let (parent, key) = path.rsplit_once('/').unwrap();
        let key = key.replace("~1", "/");
        response.pointer_mut(parent).unwrap()[key] = value;
        let (api, _, task) = server(response).await;
        assert!(
            api.review_runtime_token("controlled-projected-token")
                .await
                .is_err(),
            "{path}"
        );
        task.abort();
    }
}
