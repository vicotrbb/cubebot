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

const FIRST: &str = r#"{"login":"first-admin","password":"first administrator password"}"#;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn setup_makes_the_first_account_the_administrator_once() {
    let router = api::database_router(support::database().await);
    let (status, setup) = request(
        router.clone(),
        "GET",
        "/api/identity/setup",
        "",
        json!(null),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(setup, json!({"open": true}));
    let first: Value = serde_json::from_str(FIRST).unwrap();
    let (status, created) = request(router.clone(), "POST", "/api/identity/setup", "", first).await;
    assert_eq!(status, StatusCode::OK);
    let token = created["token"].as_str().unwrap();
    let (status, me) = request(
        router.clone(),
        "GET",
        "/api/identity/me",
        token,
        json!(null),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        (me["login"].as_str(), me["administrator"].as_bool()),
        (Some("first-admin"), Some(true))
    );
    let second = json!({"login":"second-admin","password":"second administrator password"});
    let (status, _) = request(router.clone(), "POST", "/api/identity/setup", "", second).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let (_, setup) = request(router, "GET", "/api/identity/setup", "", json!(null)).await;
    assert_eq!(setup, json!({"open": false}));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn setup_closes_after_the_cli_bootstrap() {
    let pool = support::database().await;
    cubebot_orchestrator::identity::Identity::new(pool.clone())
        .bootstrap("cli-admin", "cli administrator password")
        .await
        .unwrap();
    let router = api::database_router(pool);
    let (_, setup) = request(
        router.clone(),
        "GET",
        "/api/identity/setup",
        "",
        json!(null),
    )
    .await;
    assert_eq!(setup, json!({"open": false}));
    let first: Value = serde_json::from_str(FIRST).unwrap();
    let (status, _) = request(router, "POST", "/api/identity/setup", "", first).await;
    assert_eq!(status, StatusCode::CONFLICT);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn concurrent_setups_create_one_administrator() {
    let pool = support::database().await;
    let router = api::database_router(pool.clone());
    let attempt = |login: &str| {
        let body = json!({"login": login, "password": "racing administrator password"});
        request(router.clone(), "POST", "/api/identity/setup", "", body)
    };
    let ((first, _), (second, _)) = tokio::join!(attempt("racer-one"), attempt("racer-two"));
    let mut statuses = [first, second];
    statuses.sort();
    assert_eq!(statuses, [StatusCode::OK, StatusCode::CONFLICT]);
    let accounts: i64 = sqlx::query_scalar!(r#"SELECT count(*) AS "count!" FROM human_accounts"#)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(accounts, 1);
}
