use crate::runtime_support::fixture;
use axum::{Extension, Router, body::Body, http::Request};
use cubebot_orchestrator::{identity::Identity, storage::Storage};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

struct Reply {
    status: u16,
    body: Vec<u8>,
}

impl Reply {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }
}

async fn send(router: &Router, token: &str, method: &str, path: &str, body: Body) -> Reply {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(body)
        .unwrap();
    let response = router.clone().oneshot(request).await.unwrap();
    Reply {
        status: response.status().as_u16(),
        body: response
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    }
}

async fn call(router: &Router, token: &str, method: &str, path: &str, body: Value) -> Reply {
    send(router, token, method, path, Body::from(body.to_string())).await
}

async fn get(router: &Router, token: &str, path: &str) -> Reply {
    send(router, token, "GET", path, Body::empty()).await
}

async fn new_page(
    router: &Router,
    token: &str,
    prefix: &str,
    title: &str,
    parent: Value,
) -> String {
    let body = json!({"request_id":Uuid::new_v4(),"parent":parent,"kind":"page","title":title,"body":"# Start\n\nfirst words"});
    let created = call(router, token, "POST", &format!("{prefix}/pages"), body).await;
    assert_eq!(created.status, 201, "{:?}", created.json());
    created.json()["id"].as_str().unwrap().to_owned()
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn people_and_agents_share_one_wiki_within_the_agents_capability() {
    let f = fixture().await;
    let identity = Identity::new(f.pool.clone());
    let admin = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let administrator = identity.authenticate(&admin.token).await.unwrap();
    identity
        .create_account(
            &administrator,
            "member",
            "member password",
            false,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    let member = identity
        .login("member", "member password")
        .await
        .unwrap()
        .token;
    let directory = std::env::temp_dir().join(format!("cubebot-kb-{}", Uuid::new_v4()));
    let storage = Storage::start(f.pool.clone(), directory.clone(), 1 << 30)
        .await
        .unwrap();
    let console = cubebot_orchestrator::api::configured_router(f.pool.clone(), None, None)
        .await
        .unwrap()
        .layer(Extension(storage.clone()));
    let token = f
        .auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap()
        .token;
    let runtime =
        cubebot_orchestrator::api::runtime_router(f.auth.clone()).layer(Extension(storage.clone()));
    assert_eq!(
        get(&console, "not-a-session", "/api/kb/pages").await.status,
        401
    );

    let folder = call(
        &console,
        &member,
        "POST",
        "/api/kb/pages",
        json!({"request_id":Uuid::new_v4(),"parent":null,"kind":"folder","title":"Guides"}),
    )
    .await
    .json();
    let folder = folder["id"].as_str().unwrap();
    let page = new_page(&console, &member, "/api/kb", "Deploying", json!(folder)).await;
    let request = Uuid::new_v4();
    let again = json!({"request_id":request,"parent":null,"kind":"page","title":"Once"});
    let first = call(
        &runtime,
        &token,
        "POST",
        "/api/runtime/kb/pages",
        again.clone(),
    )
    .await;
    let second = call(&runtime, &token, "POST", "/api/runtime/kb/pages", again).await;
    assert_eq!(first.json()["id"], second.json()["id"]);
    assert!(first.json()["updated_by"]["agent"].is_string());
    let outline = get(&runtime, &token, "/api/runtime/kb/pages").await.json();
    assert_eq!(outline.as_array().unwrap().len(), 3);
    assert!(outline[0].get("body").is_none());

    let path = format!("/api/runtime/kb/pages/{page}");
    let replaced = call(
        &runtime,
        &token,
        "PATCH",
        &path,
        json!({"replace":{"old_text":"first words","new_text":"agent words"}}),
    )
    .await;
    assert_eq!(replaced.status, 200);
    assert_eq!(replaced.json()["revision"], 2);
    assert_eq!(replaced.json()["body"], "# Start\n\nagent words");
    let missing = call(
        &runtime,
        &token,
        "PATCH",
        &path,
        json!({"replace":{"old_text":"first words","new_text":"x"}}),
    )
    .await;
    assert_eq!(missing.status, 400);
    let console_path = format!("/api/kb/pages/{page}");
    let stale = call(
        &console,
        &member,
        "PATCH",
        &console_path,
        json!({"expected_revision":1,"body":"mine"}),
    )
    .await;
    assert_eq!(stale.status, 409);
    assert_eq!(stale.json()["revision"], 2);
    let blind = call(
        &console,
        &member,
        "PATCH",
        &console_path,
        json!({"body":"x"}),
    )
    .await;
    assert_eq!(blind.status, 400);
    let saved = call(
        &console,
        &member,
        "PATCH",
        &console_path,
        json!({"expected_revision":2,"body":"mine at last","title":"Deploying it"}),
    )
    .await;
    assert_eq!(saved.status, 200);
    assert!(saved.json()["updated_by"]["account"].is_string());

    let hits = get(&console, &member, "/api/kb/search?q=last").await.json();
    assert_eq!(hits[0]["id"], json!(page));
    let hits = get(&console, &member, "/api/kb/search?q=Deplo")
        .await
        .json();
    assert_eq!(hits[0]["id"], json!(page));
    let wild = get(&console, &member, "/api/kb/search?q=%25").await.json();
    assert_eq!(wild, json!([]));

    let versions = get(&console, &member, &format!("{console_path}/versions"))
        .await
        .json();
    assert_eq!(versions.as_array().unwrap().len(), 2);
    assert_eq!(versions[0]["by"]["name"], "member");
    let earlier = versions[1]["revision"].as_i64().unwrap();
    let old = get(
        &console,
        &member,
        &format!("{console_path}/versions/{earlier}"),
    )
    .await
    .json();
    assert_eq!(old["body"], "# Start\n\nagent words");

    let child = new_page(&console, &member, "/api/kb", "Child", json!(page)).await;
    let cycle = call(
        &console,
        &member,
        "POST",
        &format!("{console_path}/place"),
        json!({"parent":child,"after":null}),
    )
    .await;
    assert_eq!(cycle.status, 400);
    let other = new_page(&console, &member, "/api/kb", "Other", json!(page)).await;
    let last = new_page(&console, &member, "/api/kb", "Last", json!(page)).await;
    let moved = call(
        &console,
        &member,
        "POST",
        &format!("/api/kb/pages/{last}/place"),
        json!({"parent":page,"after":child}),
    )
    .await;
    assert_eq!(moved.status, 204);
    let tree = get(&console, &member, "/api/kb/pages").await.json();
    let order: Vec<&str> = tree
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["parent"] == json!(page))
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    assert_eq!(order, [child.as_str(), last.as_str(), other.as_str()]);

    let attach = |prefix: &str, name: &str| {
        format!(
            "{prefix}/pages/{page}/attachments?name={name}&request_id={}",
            Uuid::new_v4()
        )
    };
    let upload = Request::builder()
        .method("POST")
        .uri(attach("/api/kb", "pic.png"))
        .header("authorization", format!("Bearer {member}"))
        .header("content-type", "image/png")
        .body(Body::from(&b"not really a png"[..]))
        .unwrap();
    let uploaded = console.clone().oneshot(upload).await.unwrap();
    assert_eq!(uploaded.status(), 201);
    let bytes = uploaded.into_body().collect().await.unwrap().to_bytes();
    let attachment: Value = serde_json::from_slice(&bytes).unwrap();
    let attachment = attachment["id"].as_str().unwrap();
    let read = get(
        &runtime,
        &token,
        &format!("/api/runtime/kb/attachments/{attachment}"),
    )
    .await;
    assert_eq!(read.body, b"not really a png");
    let page_read = get(&runtime, &token, &path).await.json();
    assert_eq!(page_read["attachments"][0]["content_type"], "image/png");

    let gone = call(&runtime, &token, "DELETE", &path, Value::Null).await;
    assert_eq!(gone.status, 204);
    assert_eq!(get(&console, &member, &console_path).await.status, 404);
    let attachment_path = format!("/api/kb/attachments/{attachment}");
    assert_eq!(get(&console, &member, &attachment_path).await.status, 404);
    let trash = get(&console, &member, "/api/kb/trash").await.json();
    assert_eq!(trash.as_array().unwrap().len(), 1);
    assert_eq!(trash[0]["id"], json!(page));
    let tree = get(&console, &member, "/api/kb/pages").await.json();
    assert!(
        tree.as_array()
            .unwrap()
            .iter()
            .all(|entry| entry["id"] != json!(child))
    );
    let purge = format!("/api/runtime/kb/pages/{page}/purge");
    assert_eq!(
        call(&runtime, &token, "POST", &purge, Value::Null)
            .await
            .status,
        403
    );
    let restore = format!("{console_path}/restore");
    assert_eq!(
        call(&console, &member, "POST", &restore, Value::Null)
            .await
            .status,
        204
    );
    assert_eq!(
        get(&console, &member, &format!("/api/kb/pages/{child}"))
            .await
            .status,
        200
    );

    call(&console, &member, "DELETE", &console_path, Value::Null).await;
    let purge = format!("{console_path}/purge");
    assert_eq!(
        call(&console, &member, "POST", &purge, Value::Null)
            .await
            .status,
        204
    );
    let blobs = sqlx::query_scalar!("SELECT count(*) AS \"count!\" FROM blobs")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert_eq!(blobs, 0);

    sqlx::query!("UPDATE agents SET knowledge_base = false")
        .execute(&f.pool)
        .await
        .unwrap();
    let refused = get(&runtime, &token, "/api/runtime/kb/pages").await;
    assert_eq!(refused.status, 403);
    assert_eq!(get(&console, &member, "/api/kb/pages").await.status, 200);

    std::fs::remove_dir_all(directory).unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_file_share_finds_a_person_by_login_in_any_case_and_says_why_not_otherwise() {
    let f = fixture().await;
    let directory = std::env::temp_dir().join(format!("cubebot-share-{}", Uuid::new_v4()));
    let storage = Storage::start(f.pool.clone(), directory.clone(), 1 << 30)
        .await
        .unwrap();
    let token = f
        .auth
        .bootstrap("projected-token", Uuid::new_v4())
        .await
        .unwrap()
        .token;
    let runtime =
        cubebot_orchestrator::api::runtime_router(f.auth.clone()).layer(Extension(storage));
    let query = format!(
        "/api/runtime/files?name=poem.txt&request_id={}",
        Uuid::new_v4()
    );
    let put = send(&runtime, &token, "POST", &query, Body::from("verse")).await;
    assert_eq!(put.status, 201);
    let file = put.json()["id"].as_str().unwrap().to_owned();
    let shares = format!("/api/runtime/files/{file}/shares");
    let share = |name: &str| json!({"kind":"user","name":name,"permission":"read"});
    let by_name = call(&runtime, &token, "POST", &shares, share("Victor Bona")).await;
    assert_eq!(by_name.status, 400);
    assert!(by_name.json()["error"].as_str().unwrap().contains("login"));
    let by_login = call(&runtime, &token, "POST", &shares, share("ADMIN")).await;
    assert_eq!(by_login.status, 200);
    std::fs::remove_dir_all(directory).unwrap();
}
