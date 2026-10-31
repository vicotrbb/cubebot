//! A manager sets and resets an agent's wallpaper; the agent fetches its own, against real
//! PostgreSQL.
use crate::runtime_support::fixture;
use axum::{Router, body::Body, http::Request};
use cubebot_orchestrator::{
    connections::{ConfigureProvider, Connections, Credentials},
    identity::Identity,
};
use http_body_util::BodyExt;
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "https://cubebot.test";
const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot really pixels";

async fn call(router: &Router, request: Request<Body>) -> (u16, Vec<u8>) {
    let response = router.clone().oneshot(request).await.unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (status, bytes.to_vec())
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_manager_sets_the_wallpaper_the_agent_fetches_it_and_a_reset_falls_back() {
    let f = fixture().await;
    let identity = Identity::new(f.pool.clone());
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    Connections::new(f.pool.clone(), Credentials::new([7; 32]))
        .configure(
            &admin,
            &ConfigureProvider {
                request_id: Uuid::new_v4(),
                expected_revision: 0,
                provider: cubebot_protocol::runtime::ProviderKind::Openai,
                model: "controlled-model".into(),
                context_window: Some(128_000),
                api_key: "controlled-key".into(),
                access: None,
            },
        )
        .await
        .unwrap();
    let auth = f.auth.clone().with_credentials(Credentials::new([7; 32]));
    let token = auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap()
        .token;
    let runtime = cubebot_orchestrator::api::runtime_router(auth);
    let console = cubebot_orchestrator::api::configured_router_with_credentials(
        f.pool.clone(),
        Some(ORIGIN.into()),
        None,
        None,
    )
    .await
    .unwrap();
    let path = format!("/api/agents/{}/wallpaper", f.agent.0);
    let write = |method: &str, body: &[u8]| {
        Request::builder()
            .method(method)
            .uri(&path)
            .header(
                "cookie",
                format!("__Host-cubebot_session={}", session.token),
            )
            .header("origin", ORIGIN)
            .body(Body::from(body.to_vec()))
            .unwrap()
    };
    let fetch = || {
        Request::builder()
            .uri("/api/runtime/wallpaper")
            .header("authorization", format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap()
    };

    // No wallpaper means the default; only JPEG and PNG bytes are accepted.
    assert_eq!(call(&runtime, fetch()).await.0, 404);
    assert_eq!(call(&console, write("PUT", b"GIF89a")).await.0, 400);
    let oversized = [PNG, &vec![0; 4 * 1024 * 1024]].concat();
    assert_eq!(call(&console, write("PUT", &oversized)).await.0, 413);

    assert_eq!(call(&console, write("PUT", PNG)).await.0, 204);
    assert_eq!(call(&runtime, fetch()).await, (200, PNG.to_vec()));

    assert_eq!(call(&console, write("DELETE", b"")).await.0, 204);
    assert_eq!(call(&runtime, fetch()).await.0, 404);
}
