use crate::quota;
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
    let storage = Storage::start(f.pool.clone(), directory.clone(), quota(1 << 30))
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
    let storage = Storage::start(f.pool.clone(), directory.clone(), quota(used + 10))
        .await
        .unwrap();
    assert!(!orphan.exists() && !staged.exists());
    let router = console(&f.pool, storage.clone()).await;
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
    // The per-file upload limit refuses with its own status and leaves nothing staged.
    storage.limit(None, Some(4));
    let large = upload(&router, &alice, "/api/files", &query("large"), &[1; 5]).await;
    assert_eq!(large.status, 413);
    assert_eq!(large.json()["error"], "file_too_large");
    assert_eq!(
        std::fs::read_dir(directory.join("staging"))
            .unwrap()
            .count(),
        0
    );
    storage.limit(None, None);
    let fits = upload(&router, &alice, "/api/files", &query("fits"), &[1; 10]).await;
    assert_eq!(fits.status, 201);
    // An administrator's quota applies at once, within 90% of the volume's own size.
    storage.limit(Some(u64::MAX), None);
    let capacity = storage.usage().capacity.unwrap();
    assert_eq!(storage.usage().quota, capacity / 10 * 9);
    storage.limit(None, None);
    assert_eq!(storage.usage().quota, used + 10);

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

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn administrators_change_the_name_quota_and_upload_limit_at_once() {
    let f = fixture().await;
    let directory: PathBuf =
        std::env::temp_dir().join(format!("cubebot-settings-{}", Uuid::new_v4()));
    let identity = Identity::new(f.pool.clone());
    let admin = identity
        .login("admin", "administrator password")
        .await
        .unwrap()
        .token;
    let alice = person(&identity, &admin, "alice").await;
    let storage = Storage::start(f.pool.clone(), directory.clone(), quota(1 << 30))
        .await
        .unwrap();
    let router = console(&f.pool, storage.clone()).await;
    const PATH: &str = "/api/installation/settings";

    assert_eq!(get(&router, &alice, PATH).await.status, 403);
    let current = get(&router, &admin, PATH).await.json();
    assert_eq!(current["name"], Value::Null);
    assert_eq!(current["storage"]["quota_bytes"], 1u64 << 30);
    assert_eq!(current["storage"]["default_quota_bytes"], 1u64 << 30);
    assert_eq!(current["storage"]["upload_limit_bytes"], Value::Null);
    let revision = current["revision"].as_i64().unwrap();

    let settings = |name: Value, quota: Value, upload: Value, revision: i64| json!({"name":name,"storage_quota_bytes":quota,"upload_limit_bytes":upload,"revision":revision});
    for invalid in [
        settings(json!("  "), Value::Null, Value::Null, revision),
        settings(json!("x".repeat(65)), Value::Null, Value::Null, revision),
        settings(Value::Null, json!(1024), Value::Null, revision),
        settings(Value::Null, Value::Null, json!(1024), revision),
    ] {
        let refused = json(&router, &admin, "PUT", PATH, invalid.clone()).await;
        assert_eq!(refused.status, 400, "{invalid}");
    }
    let stale = settings(json!("Acme"), Value::Null, Value::Null, revision - 1);
    assert_eq!(json(&router, &admin, "PUT", PATH, stale).await.status, 409);
    let denied = settings(json!("Acme"), Value::Null, Value::Null, revision);
    assert_eq!(json(&router, &alice, "PUT", PATH, denied).await.status, 403);

    let wanted = settings(json!(" Acme "), Value::Null, json!(1 << 20), revision);
    let saved = json(&router, &admin, "PUT", PATH, wanted).await;
    assert_eq!(saved.status, 200);
    assert_eq!(saved.json()["name"], "Acme");
    assert_eq!(saved.json()["storage"]["upload_limit_bytes"], 1 << 20);
    assert_eq!(saved.json()["revision"], revision + 1);
    assert_eq!(
        get(&router, &alice, "/api/installation").await.json()["name"],
        "Acme"
    );

    let query = |name: &str| format!("name={name}&request_id={}", Uuid::new_v4());
    let over = vec![1; (1 << 20) + 1];
    let large = upload(&router, &alice, "/api/files", &query("large"), &over).await;
    assert_eq!(large.status, 413);
    let fits = upload(&router, &alice, "/api/files", &query("fits"), &over[1..]).await;
    assert_eq!(fits.status, 201);

    // Clearing a value returns to the deployment's.
    let cleared = settings(Value::Null, Value::Null, Value::Null, revision + 1);
    assert_eq!(
        json(&router, &admin, "PUT", PATH, cleared).await.status,
        200
    );
    assert_eq!(
        get(&router, &alice, "/api/installation").await.json()["name"],
        "Cubebot"
    );
    assert_eq!(storage.usage().upload, None);
    let audited: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_log WHERE kind = 'installation.settings'")
            .fetch_one(&f.pool)
            .await
            .unwrap();
    assert_eq!(audited, 2);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn people_attach_only_their_own_uploads_and_the_agent_reads_them() {
    let f = fixture().await;
    let directory: PathBuf = std::env::temp_dir().join(format!("cubebot-files-{}", Uuid::new_v4()));
    let identity = Identity::new(f.pool.clone());
    let admin = identity
        .login("admin", "administrator password")
        .await
        .unwrap()
        .token;
    let alice = person(&identity, &admin, "alice").await;
    let storage = Storage::start(f.pool.clone(), directory, quota(1 << 30))
        .await
        .unwrap();
    let router = console(&f.pool, storage.clone()).await;
    let named = |name: &str| format!("name={name}&request_id={}", Uuid::new_v4());
    let photo = id(&upload(&router, &admin, "/api/files", &named("photo.png"), b"png").await);
    let theirs = id(&upload(&router, &alice, "/api/files", &named("a.pdf"), b"pdf").await);
    let messages = format!("/api/conversations/{}/messages", f.agent.0);
    let message = |files: Vec<&str>| json!({"request_id": Uuid::new_v4(), "message": "", "attachments": files});

    let refused = json(&router, &admin, "POST", &messages, message(vec![&theirs])).await;
    assert_eq!(refused.status, 400);
    let twice = json(
        &router,
        &admin,
        "POST",
        &messages,
        message(vec![&photo, &photo]),
    )
    .await;
    assert_eq!(twice.status, 400);
    let sent = json(&router, &admin, "POST", &messages, message(vec![&photo])).await;
    assert!(sent.status < 300, "{} {:?}", sent.status, sent.body);
    let turn = id(&sent);
    assert_eq!(
        sent.json()["message_attachments"][0]["file"],
        photo.as_str()
    );
    assert_eq!(sent.json()["attachments"], json!([]));

    let content = format!("/api/attachments/{turn}/{photo}/content");
    assert_eq!(get(&router, &admin, &content).await.body, &b"png"[..]);
    assert_eq!(get(&router, &alice, &content).await.status, 404);
    let agent = cubebot_orchestrator::storage::Principal::Agent(f.agent);
    assert!(storage.open(agent, photo.parse().unwrap()).await.is_ok());
    let agent = cubebot_orchestrator::storage::Principal::Agent(f.agent);
    assert!(storage.open(agent, theirs.parse().unwrap()).await.is_err());

    let name: String = sqlx::query_scalar("SELECT name FROM agents WHERE id=$1")
        .bind(f.agent.0)
        .fetch_one(&f.pool)
        .await
        .unwrap();
    let room = json!({"request_id": Uuid::new_v4(), "name": "Design", "agents": [f.agent.0]});
    let room = id(&json(&router, &admin, "POST", "/api/rooms", room).await);
    let chart = id(&upload(&router, &admin, "/api/files", &named("chart.png"), b"chart").await);
    let posted = json!({"request_id": Uuid::new_v4(), "message": format!("@{name} look"), "attachments": [chart]});
    let posted = json(
        &router,
        &admin,
        "POST",
        &format!("/api/rooms/{room}/messages"),
        posted,
    )
    .await;
    assert!(posted.status < 300, "{} {:?}", posted.status, posted.body);
    let message = id(&posted);
    assert_eq!(posted.json()["attachments"][0]["file"], chart.as_str());
    let content = format!("/api/attachments/{message}/{chart}/content");
    assert_eq!(get(&router, &admin, &content).await.body, &b"chart"[..]);
    assert_eq!(get(&router, &alice, &content).await.status, 404);
    let delivered: Uuid = sqlx::query_scalar(
        "SELECT x.file_id FROM turn_attachments x JOIN chat_turns t ON t.id=x.turn_id WHERE t.room_message_id=$1 AND x.inbound",
    )
    .bind(message.parse::<Uuid>().unwrap())
    .fetch_one(&f.pool)
    .await
    .unwrap();
    assert_eq!(delivered.to_string(), chart);
    let agent = cubebot_orchestrator::storage::Principal::Agent(f.agent);
    assert!(storage.open(agent, delivered).await.is_ok());
}
