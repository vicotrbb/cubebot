//! The console's memory API through the configured browser router: cookies, CSRF and grants.
use crate::support;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use cubebot_orchestrator::{
    agents::{Agents, CreateAgent},
    identity::{
        Identity,
        access::{Capability, GrantRequest, Recipient, Scope},
    },
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

const ORIGIN: &str = "https://cubebot.test";

struct Browser {
    router: axum::Router,
    cookie: String,
}

impl Browser {
    async fn call(
        &self,
        method: &str,
        path: &str,
        body: Value,
        origin: Option<&str>,
    ) -> (StatusCode, Value) {
        let mut request = Request::builder()
            .method(method)
            .uri(path)
            .header("content-type", "application/json")
            .header("cookie", &self.cookie);
        if let Some(origin) = origin {
            request = request.header("origin", origin);
        }
        let body = if body.is_null() {
            Body::empty()
        } else {
            Body::from(body.to_string())
        };
        let response = self
            .router
            .clone()
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = response.into_body().collect().await.unwrap().to_bytes();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }
    async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.call("GET", path, Value::Null, None).await
    }
    async fn write(&self, method: &str, path: &str, body: Value) -> (StatusCode, Value) {
        self.call(method, path, body, Some(ORIGIN)).await
    }
}

async fn grant(
    identity: &Identity,
    admin: &cubebot_orchestrator::identity::Actor,
    member: Uuid,
    agent: Uuid,
    capability: Capability,
) {
    identity
        .grant(
            admin,
            &GrantRequest {
                request_id: Uuid::new_v4(),
                recipient: Recipient::User { id: member },
                scope: Scope::Agent { id: agent },
                capability,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn the_console_reads_and_edits_memories_only_with_grants_and_fresh_revisions() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let admin_session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&admin_session.token).await.unwrap();
    identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let member_session = identity.login("member", "member password").await.unwrap();
    let member_actor = identity.authenticate(&member_session.token).await.unwrap();
    let agents = Agents::new(pool.clone());
    let create = |name: &str| CreateAgent {
        request_id: Uuid::new_v4(),
        name: name.into(),
        instructions: String::new(),
    };
    let writer = agents.create(&admin, &create("Writer")).await.unwrap().id;
    let other = agents.create(&admin, &create("Other")).await.unwrap().id;
    let router = cubebot_orchestrator::api::configured_router_with_credentials(
        pool.clone(),
        Some(ORIGIN.into()),
        None,
        None,
    )
    .await
    .unwrap();
    let as_admin = Browser {
        router: router.clone(),
        cookie: format!("__Host-cubebot_session={}", admin_session.token),
    };
    let as_member = Browser {
        router: router.clone(),
        cookie: format!("__Host-cubebot_session={}", member_session.token),
    };
    let anonymous = Browser {
        router,
        cookie: String::new(),
    };
    let memories = format!("/api/agents/{writer}/memories");

    assert_eq!(anonymous.get(&memories).await.0, StatusCode::UNAUTHORIZED);
    // A cookie-authenticated write without the browser origin is cross-site and refused.
    let note = json!({"content":"The deadline is November 15"});
    assert_eq!(
        as_admin.call("POST", &memories, note.clone(), None).await.0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        as_admin
            .call("POST", &memories, note.clone(), Some("http://evil.test"))
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    let (status, created) = as_admin.write("POST", &memories, note).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        (
            created["revision"].as_i64(),
            created["edited_by_human"].as_bool()
        ),
        (Some(1), Some(true))
    );
    assert_eq!(created["source_turn"], Value::Null);
    let id = created["id"].as_str().unwrap().to_owned();
    let item = format!("{memories}/{id}");
    as_admin
        .write("POST", &memories, json!({"content":"Prefers metric units"}))
        .await;

    let (status, found) = as_admin.get(&format!("{memories}?query=deadlines")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(found.as_array().unwrap().len(), 1);
    assert_eq!(as_admin.get(&memories).await.1.as_array().unwrap().len(), 2);

    // Edits name the revision they started from; a stale one changes nothing.
    let (status, revised) = as_admin
        .write(
            "PUT",
            &item,
            json!({"content":"The deadline is November 22","expected_revision":1}),
        )
        .await;
    assert_eq!(
        (status, revised["revision"].as_i64()),
        (StatusCode::OK, Some(2))
    );
    let stale = json!({"content":"The deadline is November 29","expected_revision":1});
    assert_eq!(
        as_admin.write("PUT", &item, stale).await.0,
        StatusCode::CONFLICT
    );
    assert_eq!(
        as_admin
            .write(
                "DELETE",
                &format!("{item}?expected_revision=1"),
                Value::Null
            )
            .await
            .0,
        StatusCode::CONFLICT
    );
    // A memory is reachable only under the agent that owns it.
    let elsewhere = format!("/api/agents/{other}/memories/{id}");
    let moved = json!({"content":"Hijacked","expected_revision":2});
    assert_ne!(
        as_admin.write("PUT", &elsewhere, moved).await.0,
        StatusCode::OK
    );
    assert!(
        as_admin
            .get(&format!("/api/agents/{other}/memories"))
            .await
            .1
            .as_array()
            .unwrap()
            .is_empty()
    );

    // Invalid input is refused before it is stored.
    for body in [
        json!({"content":"   "}),
        json!({"content":"x".repeat(4097)}),
        json!({"content":"a\u{0}b"}),
    ] {
        assert_eq!(
            as_admin.write("POST", &memories, body).await.0,
            StatusCode::BAD_REQUEST
        );
    }
    assert!(
        as_admin
            .write("POST", &memories, json!({"content":"x","agent":other}))
            .await
            .0
            .is_client_error()
    );
    assert_eq!(
        as_admin
            .get(&format!("{memories}?query={}", "q".repeat(1025)))
            .await
            .0,
        StatusCode::BAD_REQUEST
    );

    // Members need memory.read to see and memory.edit to change, per agent.
    assert_eq!(as_member.get(&memories).await.0, StatusCode::NOT_FOUND);
    grant(
        &identity,
        &admin,
        member_actor.id,
        writer,
        Capability::MemoryRead,
    )
    .await;
    let (status, seen) = as_member.get(&memories).await;
    assert_eq!(
        (status, seen.as_array().unwrap().len()),
        (StatusCode::OK, 2)
    );
    assert_eq!(
        as_member
            .get(&format!("/api/agents/{other}/memories"))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    let edit = json!({"content":"Member edit","expected_revision":2});
    assert_eq!(
        as_member
            .write("POST", &memories, json!({"content":"Member note"}))
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        as_member.write("PUT", &item, edit.clone()).await.0,
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        as_member
            .write(
                "DELETE",
                &format!("{item}?expected_revision=2"),
                Value::Null
            )
            .await
            .0,
        StatusCode::NOT_FOUND
    );
    grant(
        &identity,
        &admin,
        member_actor.id,
        writer,
        Capability::MemoryEdit,
    )
    .await;
    let (status, edited) = as_member.write("PUT", &item, edit).await;
    assert_eq!(
        (status, edited["revision"].as_i64()),
        (StatusCode::OK, Some(3))
    );

    assert_eq!(
        as_admin
            .write(
                "DELETE",
                &format!("{item}?expected_revision=3"),
                Value::Null
            )
            .await
            .0,
        StatusCode::NO_CONTENT
    );
    let left = as_admin.get(&memories).await.1;
    assert_eq!(left.as_array().unwrap().len(), 1);
    assert_eq!(left[0]["content"], "Prefers metric units");
    let revisions: Vec<(i64, bool, Option<Uuid>)> = sqlx::query_as("SELECT revision,deleted,human_id FROM memory_revisions WHERE memory_id=$1::uuid ORDER BY revision")
        .bind(&id).fetch_all(&pool).await.unwrap();
    assert_eq!(
        revisions,
        [
            (1, false, Some(admin.id)),
            (2, false, Some(admin.id)),
            (3, false, Some(member_actor.id)),
            (4, true, Some(admin.id))
        ]
    );
}
