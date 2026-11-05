use crate::runtime_support::fixture;
use axum::{
    Extension, Router,
    body::{Body, Bytes},
    http::Request,
};
use cubebot_orchestrator::{identity::Identity, storage::Storage};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use tower::ServiceExt;
use uuid::Uuid;

struct Reply {
    status: u16,
    headers: axum::http::HeaderMap,
    body: Bytes,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

async fn send(
    router: &Router,
    token: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Body,
) -> Reply {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"));
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let response = router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    Reply {
        status: response.status().as_u16(),
        headers: response.headers().clone(),
        body: response.into_body().collect().await.unwrap().to_bytes(),
    }
}

async fn get(router: &Router, token: &str, path: &str) -> Reply {
    send(router, token, "GET", path, &[], Body::empty()).await
}

async fn json(router: &Router, token: &str, method: &str, path: &str, body: Value) -> Reply {
    let headers = [("content-type", "application/json")];
    send(
        router,
        token,
        method,
        path,
        &headers,
        Body::from(body.to_string()),
    )
    .await
}

async fn upload(router: &Router, token: &str, prefix: &str, query: &str, bytes: &[u8]) -> Reply {
    let path = format!("{prefix}?{query}");
    let headers = [("content-type", "application/pdf")];
    send(
        router,
        token,
        "POST",
        &path,
        &headers,
        Body::from(bytes.to_vec()),
    )
    .await
}

fn id(reply: &Reply) -> String {
    reply.json()["id"].as_str().unwrap().to_owned()
}

fn blobs(directory: &Path) -> usize {
    std::fs::read_dir(directory.join("blobs")).unwrap().count()
}

async fn person(identity: &Identity, admin: &str, login: &str) -> String {
    let admin = identity.authenticate(admin).await.unwrap();
    let password = format!("{login} password");
    identity
        .create_account(&admin, login, &password, false, Uuid::new_v4())
        .await
        .unwrap();
    identity.login(login, &password).await.unwrap().token
}

async fn console(pool: &sqlx::PgPool, storage: Storage) -> Router {
    cubebot_orchestrator::api::configured_router(pool.clone(), None, None)
        .await
        .unwrap()
        .layer(Extension(storage))
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn files_round_trip_stay_in_scope_and_survive_a_crash_before_commit() {
    let f = fixture().await;
    let directory: PathBuf = std::env::temp_dir().join(format!("cubebot-files-{}", Uuid::new_v4()));
    let identity = Identity::new(f.pool.clone());
    let admin = identity
        .login("admin", "administrator password")
        .await
        .unwrap()
        .token;
    let alice = person(&identity, &admin, "alice").await;
    let bob = person(&identity, &admin, "bob").await;
    let storage = Storage::start(f.pool.clone(), directory.clone(), 1 << 30)
        .await
        .unwrap();
    let router = console(&f.pool, storage.clone()).await;

    let report: Vec<u8> = (0..3_000_017u32).map(|i| (i * 31 % 251) as u8).collect();
    let request = Uuid::new_v4();
    let query = format!("name=report.pdf&request_id={request}");
    let created = upload(&router, &alice, "/api/files", &query, &report).await;
    assert_eq!(created.status, 201);
    let file = id(&created);
    assert_eq!(created.json()["size"], report.len());
    assert_eq!(created.json()["access"], "manage");
    let retried = upload(&router, &alice, "/api/files", &query, b"other bytes").await;
    assert_eq!((retried.status, id(&retried)), (200, file.clone()));
    assert_eq!(blobs(&directory), 1);

    let content = format!("/api/files/{file}/content");
    let downloaded = get(&router, &alice, &content).await;
    assert_eq!(downloaded.status, 200);
    assert!(downloaded.body == report, "downloaded bytes differ");
    assert_eq!(downloaded.headers["content-type"], "application/pdf");
    assert_eq!(downloaded.headers["x-content-type-options"], "nosniff");
    assert!(
        downloaded.headers["content-disposition"]
            .to_str()
            .unwrap()
            .starts_with("attachment;")
    );
    let range = [("range", "bytes=10-19")];
    let part = send(&router, &alice, "GET", &content, &range, Body::empty()).await;
    assert_eq!(part.status, 206);
    assert_eq!(part.body, report[10..20]);
    assert_eq!(
        part.headers["content-range"],
        format!("bytes 10-19/{}", report.len()).as_str()
    );
    let past = [("range", "bytes=9000000-")];
    let past = send(&router, &alice, "GET", &content, &past, Body::empty()).await;
    assert_eq!(past.status, 416);

    assert_eq!(get(&router, &bob, &content).await.status, 403);
    let renamed = json(
        &router,
        &bob,
        "PATCH",
        &format!("/api/files/{file}"),
        json!({"parent":null,"name":"x"}),
    )
    .await;
    assert_eq!(renamed.status, 403);
    assert_eq!(get(&router, &bob, "/api/files").await.json(), json!([]));
    let unknown = format!("/api/files/{}/content", Uuid::new_v4());
    assert_eq!(get(&router, &bob, &unknown).await.status, 404);

    let folder = json(
        &router,
        &alice,
        "POST",
        "/api/files/folders",
        json!({"parent":null,"name":"Reports"}),
    )
    .await;
    assert_eq!(folder.status, 201);
    let folder = id(&folder);
    let duplicate = json(
        &router,
        &alice,
        "POST",
        "/api/files/folders",
        json!({"name":"Reports"}),
    )
    .await;
    assert_eq!(duplicate.status, 409);
    let moved = json(
        &router,
        &alice,
        "PATCH",
        &format!("/api/files/{file}"),
        json!({"parent":folder,"name":"q3.pdf"}),
    )
    .await;
    assert_eq!(
        (moved.status, moved.json()["name"].clone()),
        (200, json!("q3.pdf"))
    );
    let into_itself = json(
        &router,
        &alice,
        "PATCH",
        &format!("/api/files/{folder}"),
        json!({"parent":folder,"name":"Reports"}),
    )
    .await;
    assert_eq!(into_itself.status, 400);
    let shares = format!("/api/files/{folder}/shares");
    let shared = json(
        &router,
        &alice,
        "PUT",
        &shares,
        json!([{"kind":"user","name":"bob","permission":"read"}]),
    )
    .await;
    assert_eq!(shared.status, 200);
    let nobody = json(
        &router,
        &alice,
        "PUT",
        &shares,
        json!([{"kind":"user","name":"nobody","permission":"read"}]),
    )
    .await;
    assert_eq!(nobody.status, 400);
    assert_eq!(get(&router, &bob, &shares).await.status, 403);
    let with_bob = get(&router, &bob, "/api/files/shared").await.json();
    assert_eq!(with_bob[0]["id"], folder.as_str());
    assert_eq!(with_bob[0]["access"], "read");
    let inside = get(&router, &bob, &format!("/api/files?parent={folder}"))
        .await
        .json();
    assert_eq!(inside[0]["id"], file.as_str());
    assert!(get(&router, &bob, &content).await.body == report);
    let query = format!("parent={folder}&name=bob.txt&request_id={}", Uuid::new_v4());
    assert_eq!(
        upload(&router, &bob, "/api/files", &query, b"no")
            .await
            .status,
        403
    );
    let delete = send(
        &router,
        &bob,
        "DELETE",
        &format!("/api/files/{file}"),
        &[],
        Body::empty(),
    )
    .await;
    assert_eq!(delete.status, 403);

    let token = f
        .auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap()
        .token;
    let runtime =
        cubebot_orchestrator::api::runtime_router(f.auth.clone()).layer(Extension(storage.clone()));
    let query = format!("name=summary.txt&request_id={}", Uuid::new_v4());
    let published = upload(
        &runtime,
        &token,
        "/api/runtime/files",
        &query,
        b"agent summary",
    )
    .await;
    assert_eq!(published.status, 201);
    let summary = id(&published);
    let agent_content = format!("/api/runtime/files/{summary}/content");
    assert_eq!(
        get(&runtime, &token, &agent_content).await.body,
        &b"agent summary"[..]
    );
    let alices = format!("/api/runtime/files/{file}/content");
    assert_eq!(get(&runtime, &token, &alices).await.status, 403);
    assert_eq!(
        get(&router, &alice, &format!("/api/files/{summary}/content"))
            .await
            .status,
        403
    );
    let share = json(
        &runtime,
        &token,
        "POST",
        &format!("/api/runtime/files/{summary}/shares"),
        json!({"kind":"user","name":"alice","permission":"read"}),
    )
    .await;
    assert_eq!(share.status, 200);
    let fetched = get(&router, &alice, &format!("/api/files/{summary}/content")).await;
    assert_eq!(fetched.body, &b"agent summary"[..]);
    let listed = get(&runtime, &token, "/api/runtime/files").await.json();
    assert_eq!(listed[0]["id"], summary.as_str());
    assert_eq!(blobs(&directory), 2);

    drop((router, runtime, storage));
    let orphan = directory.join("blobs").join(Uuid::new_v4().to_string());
    std::fs::write(&orphan, vec![0; 1 << 20]).unwrap();
    let staged = directory.join("staging").join(Uuid::new_v4().to_string());
    std::fs::write(&staged, b"partial").unwrap();
    let used = report.len() as u64 + b"agent summary".len() as u64;
    let storage = Storage::start(f.pool.clone(), directory.clone(), used + 10)
        .await
        .unwrap();
    assert!(!orphan.exists() && !staged.exists());
    let router = console(&f.pool, storage).await;
    assert!(get(&router, &alice, &content).await.body == report);

    let query = |name: &str| format!("name={name}&request_id={}", Uuid::new_v4());
    let full = upload(&router, &alice, "/api/files", &query("big"), &[1; 11]).await;
    assert_eq!(full.status, 507);
    assert_eq!(
        std::fs::read_dir(directory.join("staging"))
            .unwrap()
            .count(),
        0
    );
    let fits = upload(&router, &alice, "/api/files", &query("fits"), &[1; 10]).await;
    assert_eq!(fits.status, 201);

    let removed = send(
        &router,
        &alice,
        "DELETE",
        &format!("/api/files/{folder}"),
        &[],
        Body::empty(),
    )
    .await;
    assert_eq!(removed.status, 204);
    assert_eq!(get(&router, &alice, &content).await.status, 404);
    assert_eq!(blobs(&directory), 2);
    let again = upload(&router, &alice, "/api/files", &query("again"), &report).await;
    assert_eq!(again.status, 201);
    std::fs::remove_dir_all(&directory).unwrap();
}
