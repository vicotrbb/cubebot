//! The shared skill library, installations, runtime loading and dream-proposed skills, against
//! real PostgreSQL.
use crate::runtime_support::fixture;
use axum::{Router, body::Body, http::Request};
use cubebot_orchestrator::{
    connections::{ConfigureProvider, Connections, Credentials},
    identity::{Identity, IdentityError},
    knowledge::{CreateMemory, Files, Knowledge, PublishSkill, ReviseSkill},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

fn package(name: &str, steps: &str) -> Files {
    let entry = format!("---\nname: {name}\ndescription: Steps for {name}.\n---\n{steps}\n");
    [
        ("SKILL.md".to_string(), entry),
        ("scripts/check.sh".to_string(), "df -h".to_string()),
    ]
    .into()
}

async fn call(router: &Router, token: &str, method: &str, path: &str, body: Value) -> (u16, Value) {
    let request = Request::builder()
        .method(method)
        .uri(path)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json");
    let body = if body.is_null() {
        Body::empty()
    } else {
        Body::from(body.to_string())
    };
    let response = router
        .clone()
        .oneshot(request.body(body).unwrap())
        .await
        .unwrap();
    let status = response.status().as_u16();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn skills_publish_revise_install_load_and_come_from_dreams() {
    let f = fixture().await;
    let identity = Identity::new(f.pool.clone());
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let session = identity.login("member", "member password").await.unwrap();
    let member = identity.authenticate(&session.token).await.unwrap();
    let knowledge = Knowledge::new(f.pool.clone());

    // Any active member publishes to the library; names are unique and packages validated.
    let publish = |files| PublishSkill { files };
    let skill = knowledge
        .publish_skill(&member, &publish(package("disk-triage", "Run df.")))
        .await
        .unwrap();
    assert_eq!(
        (skill.revision, skill.description.as_str()),
        (1, "Steps for disk-triage.")
    );
    let taken = knowledge
        .publish_skill(&admin, &publish(package("disk-triage", "Other.")))
        .await;
    assert!(matches!(taken, Err(IdentityError::Conflict)));
    let mut escaping = package("escape", "x");
    escaping.insert("scripts/../../etc/x".into(), "x".into());
    let refused = knowledge.publish_skill(&admin, &publish(escaping)).await;
    assert!(matches!(refused, Err(IdentityError::InvalidInput)));

    // Installing needs agent.manage on the agent; members without it see nothing.
    let denied = knowledge.install_skill(&member, f.agent, skill.id, 1).await;
    assert!(matches!(denied, Err(IdentityError::NotFound)));
    let missing = knowledge.install_skill(&admin, f.agent, skill.id, 9).await;
    assert!(matches!(missing, Err(IdentityError::NotFound)));
    knowledge
        .install_skill(&admin, f.agent, skill.id, 1)
        .await
        .unwrap();

    // A revision names the one it read and keeps the name; installations stay pinned.
    let revise = |files, expected_revision| ReviseSkill {
        files,
        expected_revision,
    };
    let revised = knowledge
        .revise_skill(
            &member,
            skill.id,
            &revise(package("disk-triage", "Run df -h."), 1),
        )
        .await
        .unwrap();
    assert_eq!(revised.revision, 2);
    let stale = knowledge
        .revise_skill(
            &admin,
            skill.id,
            &revise(package("disk-triage", "Late."), 1),
        )
        .await;
    assert!(matches!(stale, Err(IdentityError::Conflict)));
    let renamed = knowledge
        .revise_skill(&admin, skill.id, &revise(package("disk-check", "x"), 2))
        .await;
    assert!(matches!(renamed, Err(IdentityError::InvalidInput)));
    let installed = knowledge.installed_skills(&admin, f.agent).await.unwrap();
    assert_eq!(
        (installed[0].revision, installed[0].current_revision),
        (1, 2)
    );
    let first = knowledge
        .skill_revision(&member, skill.id, 1)
        .await
        .unwrap();
    assert!(first.files["SKILL.md"].ends_with("Run df.\n"));
    assert_eq!(first.human, Some(member.id));

    // The agent loads only its installed revision.
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
    let router = cubebot_orchestrator::api::runtime_router(auth);
    let (status, loaded) = call(
        &router,
        &token,
        "GET",
        "/api/runtime/skills/disk-triage",
        Value::Null,
    )
    .await;
    assert_eq!((status, loaded["revision"].as_i64()), (200, Some(1)));
    assert_eq!(loaded["files"]["scripts/check.sh"], "df -h");
    let unknown = call(
        &router,
        &token,
        "GET",
        "/api/runtime/skills/log-rotation",
        Value::Null,
    )
    .await;
    assert_eq!(unknown.0, 404);

    // A dream sees its installed packages and publishes new and revised skills for itself.
    knowledge
        .create(
            &admin,
            f.agent,
            &CreateMemory {
                content: "Disks fill up on Mondays".into(),
            },
        )
        .await
        .unwrap();
    let run = Uuid::new_v4();
    let (status, dream) = call(
        &router,
        &token,
        "POST",
        &format!("/api/runtime/dreams/{run}"),
        Value::Null,
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(dream["skills"][0]["revision"], 1);
    assert!(dream["turns"].is_array());
    let skills = format!("/api/runtime/dreams/{run}/skills");
    let propose =
        |files: Files, expected: Option<i64>| json!({"expected_revision":expected,"files":files});
    // Stale against the library's revision 2, then a taken name: both dropped.
    let dropped = propose(package("disk-triage", "Stale."), Some(1));
    assert_eq!(call(&router, &token, "POST", &skills, dropped).await.0, 204);
    let duplicate = propose(package("disk-triage", "Again."), None);
    assert_eq!(
        call(&router, &token, "POST", &skills, duplicate).await.0,
        204
    );
    let created = propose(package("log-rotation", "Rotate."), None);
    assert_eq!(call(&router, &token, "POST", &skills, created).await.0, 204);
    let improved = propose(package("disk-triage", "Run df -h, then du."), Some(2));
    assert_eq!(
        call(&router, &token, "POST", &skills, improved).await.0,
        204
    );
    let invalid = json!({"expected_revision":null,"files":{"x.sh":"x"}});
    assert_eq!(call(&router, &token, "POST", &skills, invalid).await.0, 400);
    let end = json!({"outcome":"completed","report":"Wrote log-rotation."});
    let path = format!("/api/runtime/dreams/{run}");
    assert_eq!(call(&router, &token, "PUT", &path, end).await.0, 204);
    let late = propose(package("late-skill", "x"), None);
    assert_eq!(call(&router, &token, "POST", &skills, late).await.0, 409);

    let runs = knowledge.dreams(&admin, f.agent).await.unwrap();
    assert_eq!(runs[0].outcome.as_deref(), Some("completed"));
    let published: Vec<_> = runs[0]
        .skills
        .iter()
        .map(|skill| {
            (
                skill.name.as_str(),
                skill.revision,
                skill.previous.is_some(),
            )
        })
        .collect();
    assert_eq!(
        published,
        [("log-rotation", 1, false), ("disk-triage", 3, true)]
    );
    assert!(runs[0].skills[1].content.contains("then du"));
    let installed = knowledge.installed_skills(&admin, f.agent).await.unwrap();
    let pinned: Vec<_> = installed
        .iter()
        .map(|skill| (skill.name.as_str(), skill.revision))
        .collect();
    assert_eq!(pinned, [("disk-triage", 3), ("log-rotation", 1)]);
    let third = knowledge
        .skill_revision(&member, skill.id, 3)
        .await
        .unwrap();
    assert_eq!((third.human, third.dream_run), (None, Some(run)));

    // A person edits a dream's revision like any other; uninstalling keeps the library entry.
    let edited = knowledge
        .revise_skill(
            &member,
            skill.id,
            &revise(package("disk-triage", "Run du first."), 3),
        )
        .await
        .unwrap();
    assert_eq!(edited.revision, 4);
    let log = installed[1].skill;
    knowledge
        .uninstall_skill(&admin, f.agent, log)
        .await
        .unwrap();
    assert_eq!(
        knowledge
            .installed_skills(&admin, f.agent)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(knowledge.skills(&member).await.unwrap().len(), 2);
    f.task.abort();
}
