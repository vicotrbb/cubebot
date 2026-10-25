use axum::{body::Body, http::Request};
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn reports_process_liveness_without_claiming_readiness() {
    let response = cubebot_orchestrator::api::router()
        .oneshot(
            Request::get("/api/health/live")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.headers()["content-type"], "application/json");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
        serde_json::json!({ "status": "alive" })
    );

    let response = cubebot_orchestrator::api::router()
        .oneshot(Request::get("/api/agents").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 404);
}
