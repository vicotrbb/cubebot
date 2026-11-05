use crate::runtime_support::fixture;
use axum::{Form, Json, Router, body::Body, http::Request, routing::post};
use cubebot_orchestrator::{
    connections::{ConfigureProvider, Connections, Credentials},
    identity::{Identity, IdentityError},
};
use cubebot_protocol::runtime::ProviderKind;
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tower::ServiceExt;
use uuid::Uuid;

/// xAI's ID token for the fake account; its signature is never checked (it comes straight from the token endpoint).
const ID_TOKEN: &str = "eyJhbGciOiJFUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJodHRwczovL2F1dGgueC5haSIsImF1ZCI6ImIxYTAwNDkyLTA3M2EtNDdlYS04MTZmLTRjMzI5MjY0YTgyOCIsInN1YiI6Imdyb2stdXNlci0xIiwiZW1haWwiOiJwZXJzb25AZXhhbXBsZS5jb20iLCJpYXQiOjE3OTAwMDAwMDAsImV4cCI6NDEwMjQ0NDgwMH0.c2lnbmF0dXJl";

/// A controlled xAI: approves the device on its second poll, rotates refresh tokens, and refuses
/// a refresh token once `refuse` is set.
#[derive(Clone, Default)]
struct Vendor {
    polls: Arc<AtomicUsize>,
    refreshes: Arc<AtomicUsize>,
    revoked: Arc<AtomicUsize>,
    refuse: Arc<std::sync::atomic::AtomicBool>,
    presented: Arc<std::sync::Mutex<Vec<String>>>,
}

async fn token(
    axum::extract::State(vendor): axum::extract::State<Vendor>,
    Form(form): Form<HashMap<String, String>>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    assert_eq!(form["client_id"], "b1a00492-073a-47ea-816f-4c329264a828");
    if form["grant_type"] == "urn:ietf:params:oauth:grant-type:device_code" {
        if vendor.polls.fetch_add(1, Ordering::SeqCst) == 0 {
            return (
                axum::http::StatusCode::BAD_REQUEST,
                Json(json!({"error":"authorization_pending"})),
            )
                .into_response();
        }
        return Json(json!({"access_token":"access-1","refresh_token":"refresh-1","id_token":ID_TOKEN,"expires_in":21600,"token_type":"Bearer"})).into_response();
    }
    assert_eq!(form["grant_type"], "refresh_token");
    vendor
        .presented
        .lock()
        .unwrap()
        .push(form["refresh_token"].clone());
    if vendor.refuse.load(Ordering::SeqCst) {
        return (
            axum::http::StatusCode::BAD_REQUEST,
            Json(json!({"error":"invalid_grant"})),
        )
            .into_response();
    }
    // Long enough that a second refresher would arrive while this one is still running.
    tokio::time::sleep(Duration::from_millis(300)).await;
    let n = vendor.refreshes.fetch_add(1, Ordering::SeqCst) + 2;
    Json(json!({"access_token":format!("access-{n}"),"refresh_token":format!("refresh-{n}"),"expires_in":21600})).into_response()
}

async fn serve(vendor: Vendor) -> String {
    let router = Router::new()
        .route(
            "/oauth2/device/code",
            post(|| async {
                Json(json!({"device_code":"device-secret","user_code":"ABCD-EFGH","verification_uri":"http://127.0.0.1/device","expires_in":600,"interval":1}))
            }),
        )
        .route("/oauth2/token", post(token))
        .route(
            "/oauth2/revoke",
            post(|axum::extract::State(vendor): axum::extract::State<Vendor>| async move {
                vendor.revoked.fetch_add(1, Ordering::SeqCst);
            }),
        )
        .with_state(vendor);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}/", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    origin
}

async fn runtime_key(router: &Router, token: &str) -> (u16, Value) {
    let request = Request::builder()
        .uri("/api/runtime/provider")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_signed_in_plan_serves_only_its_owner_and_rotates_its_refresh_token_once_at_a_time() {
    let f = fixture().await;
    let identity = Identity::new(f.pool.clone());
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    identity
        .create_account(
            &admin,
            "other",
            "other administrator password",
            true,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    let session = identity
        .login("other", "other administrator password")
        .await
        .unwrap();
    let other = identity.authenticate(&session.token).await.unwrap();
    let vendor = Vendor::default();
    let origin = serve(vendor.clone()).await;
    let connections = Connections::new(f.pool.clone(), Credentials::new([7; 32]))
        .with_vendor_for_loopback_tests(&origin);

    // Device sign-in: pending until the person approves, then saved for them.
    let started = connections.start_supergrok(&admin).await.ok().unwrap();
    assert_eq!(started.user_code, "ABCD-EFGH");
    tokio::time::sleep(Duration::from_millis(1100)).await;
    assert!(
        connections
            .poll_supergrok(&admin, started.sign_in)
            .await
            .ok()
            .unwrap()
            .is_none()
    );
    // Asking again before the interval passes does not reach xAI.
    assert!(
        connections
            .poll_supergrok(&admin, started.sign_in)
            .await
            .ok()
            .unwrap()
            .is_none()
    );
    assert_eq!(vendor.polls.load(Ordering::SeqCst), 1);
    assert!(
        connections
            .poll_supergrok(&other, started.sign_in)
            .await
            .is_err()
    );
    tokio::time::sleep(Duration::from_millis(1100)).await;
    let signed = connections
        .poll_supergrok(&admin, started.sign_in)
        .await
        .ok()
        .unwrap()
        .unwrap();
    assert_eq!(
        (signed.account.as_str(), signed.mine),
        ("person@example.com", true)
    );
    let stored: Vec<u8> = sqlx::query_scalar!("SELECT credential FROM provider_subscriptions")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    for secret in ["access-1", "refresh-1"] {
        assert!(
            !stored
                .windows(secret.len())
                .any(|window| window == secret.as_bytes())
        );
    }
    // Someone else cannot replace it, nor make it the provider in use.
    let mine = |request_id| ConfigureProvider {
        request_id,
        expected_revision: 0,
        provider: ProviderKind::Supergrok,
        model: "grok-4.7".into(),
        context_window: Some(256_000),
        api_key: None,
        access: None,
    };
    assert!(matches!(
        connections.configure(&other, &mine(Uuid::new_v4())).await,
        Err(IdentityError::Forbidden)
    ));
    let mut shared = mine(Uuid::new_v4());
    shared.access = Some(Default::default());
    assert!(matches!(
        connections.configure(&admin, &shared).await,
        Err(IdentityError::InvalidInput)
    ));
    connections
        .configure(&admin, &mine(Uuid::new_v4()))
        .await
        .unwrap();

    // The owner's agent receives the access token and its expiry; never the refresh token.
    let auth = f.auth.clone().with_credentials(Credentials::new([7; 32]));
    let issued = auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap();
    let runtime = cubebot_orchestrator::api::runtime_router(auth);
    let (status, key) = runtime_key(&runtime, &issued.token).await;
    assert_eq!(status, 200);
    assert_eq!(
        (key["provider"].as_str(), key["api_key"].as_str()),
        (Some("supergrok"), Some("access-1"))
    );
    assert!(key["api_key_expires_at"].as_i64().unwrap() > 0);
    assert!(!key.to_string().contains("refresh-1"));

    // Concurrent refreshers of a due token: one exchange, and the rotated token is the one kept.
    let due =
        "UPDATE provider_subscriptions SET refresh_after = clock_timestamp() - interval '1 second'";
    sqlx::query(due).execute(&f.pool).await.unwrap();
    let second = Connections::new(f.pool.clone(), Credentials::new([7; 32]))
        .with_vendor_for_loopback_tests(&origin);
    let (a, b, c) = tokio::join!(
        connections.refresh_due(ProviderKind::Supergrok),
        connections.refresh_due(ProviderKind::Supergrok),
        second.refresh_due(ProviderKind::Supergrok),
    );
    for refreshed in [a, b, c] {
        refreshed.unwrap();
    }
    assert_eq!(vendor.refreshes.load(Ordering::SeqCst), 1);
    let (_, key) = runtime_key(&runtime, &issued.token).await;
    assert_eq!(key["api_key"], "access-2");
    sqlx::query(due).execute(&f.pool).await.unwrap();
    connections
        .refresh_due(ProviderKind::Supergrok)
        .await
        .unwrap();
    assert_eq!(
        *vendor.presented.lock().unwrap(),
        ["refresh-1", "refresh-2"]
    );

    // Another person's agent gets nothing from this plan.
    sqlx::query!("UPDATE agents SET created_by = $1", other.id)
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(runtime_key(&runtime, &issued.token).await.0, 503);
    sqlx::query!("UPDATE agents SET created_by = $1", admin.id)
        .execute(&f.pool)
        .await
        .unwrap();

    // A deactivated owner's plan stops serving their agents.
    sqlx::query!(
        "UPDATE human_accounts SET active = false WHERE id = $1",
        admin.id
    )
    .execute(&f.pool)
    .await
    .unwrap();
    assert_eq!(runtime_key(&runtime, &issued.token).await.0, 503);
    sqlx::query!(
        "UPDATE human_accounts SET active = true WHERE id = $1",
        admin.id
    )
    .execute(&f.pool)
    .await
    .unwrap();

    // A token about to expire is never handed out.
    sqlx::query("UPDATE provider_subscriptions SET expires_at = clock_timestamp() + interval '30 seconds', refresh_after = clock_timestamp() + interval '1 hour'")
        .execute(&f.pool)
        .await
        .unwrap();
    assert_eq!(runtime_key(&runtime, &issued.token).await.0, 503);

    // A refused refresh asks for a new sign-in and stops serving the plan.
    vendor
        .refuse
        .store(true, std::sync::atomic::Ordering::SeqCst);
    sqlx::query(
        "UPDATE provider_subscriptions SET expires_at = clock_timestamp() + interval '1 hour'",
    )
    .execute(&f.pool)
    .await
    .unwrap();
    sqlx::query(due).execute(&f.pool).await.unwrap();
    connections
        .refresh_due(ProviderKind::Supergrok)
        .await
        .unwrap();
    let listed = connections.subscriptions(&admin).await.unwrap();
    assert_eq!(listed[0].problem.as_deref(), Some("sign_in_again"));
    assert_eq!(runtime_key(&runtime, &issued.token).await.0, 503);

    // Signing out revokes it and leaves agents without a provider.
    assert!(
        connections
            .sign_out(&admin, ProviderKind::Supergrok)
            .await
            .unwrap()
    );
    assert_eq!(vendor.revoked.load(Ordering::SeqCst), 1);
    assert!(connections.subscriptions(&admin).await.unwrap().is_empty());
    assert!(connections.inspect(&admin).await.unwrap().is_none());
}

async fn call(router: &Router, method: &str, path: &str, token: &str) -> (u16, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty())
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn the_console_routes_reach_sign_in_and_sign_out_for_each_plan() {
    let f = fixture().await;
    let router = cubebot_orchestrator::api::configured_router_with_credentials(
        f.pool.clone(),
        None,
        None,
        Some(Credentials::new([7; 32])),
    )
    .await
    .unwrap();
    let token = Identity::new(f.pool.clone())
        .login("admin", "administrator password")
        .await
        .unwrap()
        .token;
    let (status, started) = call(
        &router,
        "POST",
        "/api/provider/subscriptions/chatgpt",
        &token,
    )
    .await;
    assert_eq!(status, 200);
    let url = started["url"].as_str().unwrap();
    assert!(
        url.starts_with("https://auth.openai.com/api/accounts/authorize?"),
        "{url}"
    );
    assert!(url.contains("client_id=dynamic_agent_client"), "{url}");
    for plan in ["chatgpt", "supergrok"] {
        let path = format!("/api/provider/subscriptions/{plan}");
        assert_eq!(
            call(&router, "DELETE", &path, &token).await.0,
            404,
            "{plan}"
        );
    }
    // A plan whose tokens no longer open is still forgotten, without claiming the vendor revoked it.
    sqlx::query!(
        "INSERT INTO provider_subscriptions (provider, owner, account, credential, expires_at, refresh_after)
         SELECT 'supergrok', id, 'person@example.com', '\\x00', clock_timestamp(), clock_timestamp()
         FROM human_accounts WHERE login = 'admin'"
    )
    .execute(&f.pool)
    .await
    .unwrap();
    let (status, listed) = call(&router, "GET", "/api/provider/subscriptions", &token).await;
    assert_eq!(
        (status, listed[0]["account"].as_str()),
        (200, Some("person@example.com"))
    );
    assert!(listed[0].get("credential").is_none());
    let (status, out) = call(
        &router,
        "DELETE",
        "/api/provider/subscriptions/supergrok",
        &token,
    )
    .await;
    assert_eq!((status, out), (200, json!({"revoked": false})));
    let (_, listed) = call(&router, "GET", "/api/provider/subscriptions", &token).await;
    assert_eq!(listed, json!([]));
}
