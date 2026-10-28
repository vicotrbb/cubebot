use crate::support;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use cubebot_orchestrator::api;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;

async fn request(
    router: axum::Router,
    method: &str,
    path: &str,
    token: &str,
    value: Value,
) -> (StatusCode, Value) {
    let response = router
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("content-type", "application/json")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(value.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&bytes).unwrap())
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn http_login_logout_and_authorized_account_creation() {
    let pool = support::database().await;
    let identity = cubebot_orchestrator::identity::Identity::new(pool.clone());
    identity
        .bootstrap("http-admin", "http administrator password")
        .await
        .unwrap();
    let router = api::database_router(pool);
    let (status, _) = request(router.clone(), "GET", "/api/identity/me", "", json!(null)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, login) = request(
        router.clone(),
        "POST",
        "/api/identity/login",
        "",
        json!({"login":"http-admin","password":"http administrator password"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let token = login["token"].as_str().unwrap();
    let (status, me) = request(
        router.clone(),
        "GET",
        "/api/identity/me",
        token,
        json!(null),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(me["administrator"], true);
    let (status, _) = request(
        router.clone(),
        "POST",
        "/api/identity/accounts",
        token,
        json!({"request_id":uuid::Uuid::new_v4(),"login":"http-member","password":"http member password","administrator":false}),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _) = request(
        router.clone(),
        "POST",
        "/api/identity/logout",
        token,
        json!(null),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, _) = request(router, "GET", "/api/identity/me", token, json!(null)).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}
