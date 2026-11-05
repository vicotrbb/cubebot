//! The audit log, profile pictures and person broadcasts against PostgreSQL: what people do is
//! recorded once with its actor and never a secret, only administrators read it, a picture must
//! be an image, and a broadcast reaches every agent that is not retired exactly once.
use crate::support;
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use cubebot_orchestrator::{
    agents::{Agents, CreateAgent, LifecycleAction, LifecycleCommand},
    identity::{Identity, IdentityError, audit::AuditQuery},
    work::{SendMessage, Work},
};
use tower::ServiceExt;
use uuid::Uuid;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nrest of a picture";

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn the_audit_log_records_sign_ins_and_changes_for_administrators_only() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    // The failed attempt goes through the route, which is what records it.
    let refused = crate::database_router(pool.clone())
        .oneshot(
            Request::post("/api/identity/login")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"login":"admin","password":"wrong password"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    let agents = Agents::new(pool.clone());
    let created = agents
        .create(
            &admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: "Scout".into(),
                instructions: "Look around".into(),
                avatar: None,
                voice: None,
            },
        )
        .await
        .unwrap();
    identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();

    let all = identity
        .audit_log(
            &admin,
            &AuditQuery {
                before: None,
                area: None,
            },
        )
        .await
        .unwrap();
    let kinds: Vec<&str> = all.iter().map(|entry| entry.kind.as_str()).collect();
    assert_eq!(
        kinds,
        [
            "account.create",
            "agent.create",
            "session.sign_in_failed",
            "session.sign_in",
            "account.bootstrap"
        ],
        "newest first"
    );
    let agent = &all[1];
    assert_eq!(
        (
            agent.actor_name.as_deref(),
            agent.resource,
            agent.resource_name.as_deref()
        ),
        (Some("admin"), Some(created.id), Some("Scout"))
    );
    assert_eq!(all[2].detail["login"], "admin");
    assert_eq!(all[3].detail["method"], "password");
    assert!(
        !serde_json::to_string(&all[0].detail)
            .unwrap()
            .contains("member password"),
        "no password reaches the log"
    );

    let sessions = AuditQuery {
        before: None,
        area: Some("session".into()),
    };
    let sessions = identity.audit_log(&admin, &sessions).await.unwrap();
    assert_eq!(sessions.len(), 2);
    let people = AuditQuery {
        before: None,
        area: Some("account,team".into()),
    };
    assert_eq!(identity.audit_log(&admin, &people).await.unwrap().len(), 2);
    let older = AuditQuery {
        before: Some(all[1].sequence),
        area: None,
    };
    assert_eq!(identity.audit_log(&admin, &older).await.unwrap().len(), 3);
    let invalid = AuditQuery {
        before: None,
        area: Some("vault.%".into()),
    };
    assert!(matches!(
        identity.audit_log(&admin, &invalid).await,
        Err(IdentityError::InvalidInput)
    ));

    let member = identity.login("member", "member password").await.unwrap();
    let member = identity.authenticate(&member.token).await.unwrap();
    let query = AuditQuery {
        before: None,
        area: None,
    };
    assert!(matches!(
        identity.audit_log(&member, &query).await,
        Err(IdentityError::Forbidden)
    ));
    identity.logout(&session.token).await.unwrap();
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    let kinds: Vec<String> = identity
        .audit_log(&admin, &query)
        .await
        .unwrap()
        .into_iter()
        .take(3)
        .map(|entry| entry.kind)
        .collect();
    assert_eq!(
        kinds,
        ["session.sign_in", "session.sign_out", "session.sign_in"]
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_profile_picture_must_be_an_image_and_every_upload_is_a_new_revision() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    assert!(identity.avatars().await.unwrap().is_empty());
    assert!(matches!(
        identity
            .set_avatar(&admin, Some(b"<svg onload=alert(1)>"))
            .await,
        Err(IdentityError::InvalidInput)
    ));
    let oversized = [PNG, &vec![0; 1024 * 1024]].concat();
    assert!(identity.set_avatar(&admin, Some(&oversized)).await.is_err());
    let first = identity.set_avatar(&admin, Some(PNG)).await.unwrap();
    let second = identity.set_avatar(&admin, Some(PNG)).await.unwrap();
    assert!(second > first);
    let listed = identity.avatars().await.unwrap();
    assert_eq!((listed[0].account, listed[0].revision), (admin.id, second));
    assert_eq!(
        identity.avatar(admin.id).await.unwrap().as_deref(),
        Some(PNG)
    );
    identity.set_avatar(&admin, None).await.unwrap();
    assert!(identity.avatars().await.unwrap().is_empty());
    assert_eq!(identity.avatar(admin.id).await.unwrap(), None);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_broadcast_reaches_every_agent_that_is_not_retired_once() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    let agents = Agents::new(pool.clone());
    let mut ids = Vec::new();
    for name in ["Ada", "Bob", "Cy"] {
        let input = CreateAgent {
            request_id: Uuid::new_v4(),
            name: name.into(),
            instructions: String::new(),
            avatar: None,
            voice: None,
        };
        ids.push(agents.create(&admin, &input).await.unwrap());
    }
    let retire = LifecycleCommand {
        request_id: Uuid::new_v4(),
        action: LifecycleAction::Retire,
        expected_revision: ids[2].revision,
    };
    agents
        .lifecycle(
            &admin,
            cubebot_orchestrator::agents::AgentId(ids[2].id),
            &retire,
        )
        .await
        .unwrap();

    let work = Work::new(pool.clone());
    let input = SendMessage {
        request_id: Uuid::new_v4(),
        message: "Release freeze starts at noon.".into(),
    };
    let sent = work.broadcast(&admin, &input).await.unwrap();
    let delivered: Vec<&str> = sent
        .deliveries
        .iter()
        .map(|delivery| delivery.agent.as_str())
        .collect();
    assert_eq!(delivered, ["Ada", "Bob"], "every agent but the retired one");
    assert_eq!((sent.author.as_str(), sent.agent), ("admin", None));
    let again = work.broadcast(&admin, &input).await.unwrap();
    assert_eq!(
        (again.id, again.deliveries.len()),
        (sent.id, 2),
        "a retry sends nothing new"
    );
    let changed = SendMessage {
        message: "Something else".into(),
        ..input
    };
    assert!(matches!(
        work.broadcast(&admin, &changed).await,
        Err(IdentityError::Conflict)
    ));

    let text: String =
        sqlx::query_scalar("SELECT message FROM chat_turns WHERE broadcast_id=$1 LIMIT 1")
            .bind(sent.id)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        text,
        "Broadcast received from admin. Every agent already has it, so do not forward or rebroadcast it; respond only for yourself:\n\nRelease freeze starts at noon."
    );
    let feed = work.broadcasts(&admin, 0).await.unwrap();
    assert_eq!((feed.messages.len(), feed.read_sequence), (1, 0));
    work.read_broadcasts(&admin, sent.sequence).await.unwrap();
    assert_eq!(
        work.broadcasts(&admin, 0).await.unwrap().read_sequence,
        sent.sequence
    );
    assert_eq!(
        work.broadcast_sequence(&admin).await.unwrap(),
        Some(sent.sequence)
    );

    identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let member = identity.login("member", "member password").await.unwrap();
    let member = identity.authenticate(&member.token).await.unwrap();
    assert!(work.broadcasts(&member, 0).await.is_err());
    let theirs = SendMessage {
        request_id: Uuid::new_v4(),
        message: "hi all".into(),
    };
    assert!(work.broadcast(&member, &theirs).await.is_err());
    assert_eq!(work.broadcast_sequence(&member).await.unwrap(), None);

    let log = identity
        .audit_log(
            &admin,
            &AuditQuery {
                before: None,
                area: Some("broadcast".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!((log.len(), log[0].detail["agents"].as_i64()), (1, Some(2)));
}
