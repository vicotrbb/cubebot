//! Linked Google Workspace groups mirror into Cubebot groups, matched by single sign-on email.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use axum::{
    Form, Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use cubebot_orchestrator::{
    connections::Credentials,
    identity::{
        Actor, Identity, IdentityError, Role,
        google::{GoogleDirectory, SaveGoogle},
        groups::{CreateGroup, GroupSource},
    },
};
use serde_json::{Value, json};
use uuid::Uuid;

use crate::support;

const PASSWORD: &str = "long enough password";
const KEY: &str = include_str!("fixtures/google_service_account.pem");
const CLIENT: &str = "sync@cubebot-test.iam.gserviceaccount.com";
const TOKEN: &str = "ya29.test-token";

/// Group address to its (email, type, status) members, as Google would list them.
type Directory = Arc<Mutex<HashMap<String, Vec<(String, String, String)>>>>;

#[derive(Clone)]
struct Google {
    groups: Directory,
}

fn key_json() -> String {
    json!({
        "type": "service_account",
        "project_id": "cubebot-test",
        "client_email": CLIENT,
        "private_key": KEY,
    })
    .to_string()
}

fn public_key() -> Vec<u8> {
    use base64::engine::general_purpose::STANDARD;
    let body: String = KEY
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let der = STANDARD.decode(body).unwrap();
    let pair = ring::signature::RsaKeyPair::from_pkcs8(&der).unwrap();
    pair.public().as_ref().to_vec()
}

async fn token(Form(form): Form<HashMap<String, String>>) -> Response {
    let refuse = |why: &str| (StatusCode::BAD_REQUEST, Json(json!({"error": why}))).into_response();
    if form.get("grant_type").map(String::as_str)
        != Some("urn:ietf:params:oauth:grant-type:jwt-bearer")
    {
        return refuse("unsupported_grant_type");
    }
    let assertion = form.get("assertion").cloned().unwrap_or_default();
    let parts: Vec<&str> = assertion.split('.').collect();
    if parts.len() != 3 {
        return refuse("invalid_grant");
    }
    let decode = |part: &str| -> Value {
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(part).unwrap()).unwrap()
    };
    let (header, claims) = (decode(parts[0]), decode(parts[1]));
    let signed = ring::signature::UnparsedPublicKey::new(
        &ring::signature::RSA_PKCS1_2048_8192_SHA256,
        public_key(),
    )
    .verify(
        format!("{}.{}", parts[0], parts[1]).as_bytes(),
        &URL_SAFE_NO_PAD.decode(parts[2]).unwrap(),
    );
    if header["alg"] != "RS256"
        || signed.is_err()
        || claims["iss"] != CLIENT
        || claims["sub"] != "admin@example.com"
        || claims["scope"] != "https://www.googleapis.com/auth/admin.directory.group.readonly"
    {
        return refuse("invalid_grant");
    }
    Json(json!({"access_token": TOKEN, "expires_in": 3600, "token_type": "Bearer"})).into_response()
}

fn authorized(headers: &HeaderMap) -> bool {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        == Some(&format!("Bearer {TOKEN}"))
}

/// One item per page, so every listing pages.
fn page(items: Vec<Value>, field: &str, query: &HashMap<String, String>) -> Response {
    let at: usize = query
        .get("pageToken")
        .map_or(0, |token| token.parse().unwrap());
    let mut body = json!({ field: items.get(at).cloned().into_iter().collect::<Vec<_>>() });
    if at + 1 < items.len() {
        body["nextPageToken"] = json!((at + 1).to_string());
    }
    Json(body).into_response()
}

async fn members(
    State(google): State<Google>,
    Path(group): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let groups = google.groups.lock().unwrap().clone();
    let Some(listed) = groups.get(&group) else {
        return (
            StatusCode::NOT_FOUND,
            Json(json!({"error": {"message": "Resource Not Found: groupKey"}})),
        )
            .into_response();
    };
    // Without derived membership, nested groups' people would be missing.
    let derived = query.get("includeDerivedMembership").map(String::as_str) == Some("true");
    let items = listed
        .iter()
        .filter(|(_, _, status)| derived || status != "NESTED")
        .map(|(email, kind, status)| {
            json!({"email": email, "type": kind, "status": if status == "NESTED" { "ACTIVE" } else { status }})
        })
        .collect();
    page(items, "members", &query)
}

async fn groups(
    State(google): State<Google>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Response {
    if !authorized(&headers) || query.get("customer").map(String::as_str) != Some("my_customer") {
        return StatusCode::FORBIDDEN.into_response();
    }
    let mut listed: Vec<_> = google.groups.lock().unwrap().iter().map(|(email, members)| {
        json!({"email": email, "name": email.split('@').next(), "directMembersCount": members.len().to_string()})
    }).collect();
    listed.sort_by_key(|group| group["email"].as_str().unwrap().to_owned());
    page(listed, "groups", &query)
}

async fn serve_google(google: Google) -> String {
    let router = Router::new()
        .route("/token", post(token))
        .route("/admin/directory/v1/groups", get(groups))
        .route("/admin/directory/v1/groups/{group}/members", get(members))
        .with_state(google);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    origin
}

struct Fixture {
    pool: sqlx::PgPool,
    identity: Identity,
    directory: GoogleDirectory,
    google: Google,
    owner: Actor,
}

async fn sign_in(identity: &Identity, login: &str) -> Actor {
    let session = identity.login(login, PASSWORD).await.unwrap();
    identity.authenticate(&session.token).await.unwrap()
}

impl Fixture {
    async fn new() -> Self {
        let pool = support::database().await;
        let identity = Identity::new(pool.clone());
        identity.bootstrap("owner", PASSWORD).await.unwrap();
        let owner = sign_in(&identity, "owner").await;
        let google = Google {
            groups: Arc::default(),
        };
        let origin = serve_google(google.clone()).await;
        let directory = GoogleDirectory::new(pool.clone(), Some(Credentials::new([7; 32])))
            .with_endpoints(
                format!("{origin}/token"),
                format!("{origin}/admin/directory/v1"),
            );
        Self {
            pool,
            identity,
            directory,
            google,
            owner,
        }
    }

    /// An account signed in once through single sign-on with this address.
    async fn person(&self, login: &str, email: &str) -> Uuid {
        let id = self
            .identity
            .create_account_as(&self.owner, login, PASSWORD, Role::Member, Uuid::new_v4())
            .await
            .unwrap();
        sqlx::query("INSERT INTO oidc_bindings (account_id, issuer, subject, email) VALUES ($1, 'https://accounts.google.com', $2, $3)")
            .bind(id)
            .bind(login)
            .bind(email)
            .execute(&self.pool)
            .await
            .unwrap();
        id
    }

    async fn link(&self, email: &str) -> Uuid {
        self.identity
            .create_group(
                &self.owner,
                &CreateGroup {
                    request_id: Uuid::new_v4(),
                    name: email.into(),
                    source: Some(GroupSource::Google),
                    external_id: Some(email.into()),
                },
            )
            .await
            .unwrap()
            .id
    }

    fn google_has(&self, group: &str, members: &[(&str, &str, &str)]) {
        self.google.groups.lock().unwrap().insert(
            group.into(),
            members
                .iter()
                .map(|(email, kind, status)| ((*email).into(), (*kind).into(), (*status).into()))
                .collect(),
        );
    }

    async fn members(&self, group: Uuid) -> Vec<Uuid> {
        let mut members: Vec<Uuid> =
            sqlx::query_scalar("SELECT account_id FROM human_team_members WHERE team_id = $1")
                .bind(group)
                .fetch_all(&self.pool)
                .await
                .unwrap();
        members.sort();
        members
    }

    async fn role(&self, account: Uuid) -> String {
        sqlx::query_scalar("SELECT role FROM human_accounts WHERE id = $1")
            .bind(account)
            .fetch_one(&self.pool)
            .await
            .unwrap()
    }

    async fn enable(&self) {
        self.directory
            .save(&self.owner, &save(true, Some(key_json())))
            .await
            .unwrap();
    }
}

fn save(enabled: bool, key: Option<String>) -> SaveGoogle {
    SaveGoogle {
        request_id: Uuid::new_v4(),
        enabled,
        admin_email: Some("Admin@Example.com".into()),
        customer: None,
        service_account: key,
    }
}

fn sorted(mut ids: Vec<Uuid>) -> Vec<Uuid> {
    ids.sort();
    ids
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn linked_groups_mirror_google_members_by_single_sign_on_email() {
    let f = Fixture::new().await;
    let ada = f.person("ada", "ada@example.com").await;
    let bob = f.person("bob", "Bob@Example.com").await;
    let cy = f.person("cyd", "cy@example.com").await;
    let auditors = f.link("auditors@example.com").await;
    let staff = f.link("Staff@Example.com").await;
    sqlx::query("UPDATE human_teams SET role = 'auditor' WHERE id = $1")
        .bind(auditors)
        .execute(&f.pool)
        .await
        .unwrap();
    f.google_has(
        "staff@example.com",
        &[
            ("ada@example.com", "USER", "ACTIVE"),
            ("BOB@example.com", "USER", "ACTIVE"),
            // Someone with no Cubebot account, a suspended person and a nested group itself.
            ("stranger@example.com", "USER", "ACTIVE"),
            ("dee@example.com", "USER", "SUSPENDED"),
            ("team@example.com", "GROUP", "ACTIVE"),
            // A person who is in staff only through a nested group.
            ("cy@example.com", "USER", "NESTED"),
        ],
    );
    f.google_has(
        "auditors@example.com",
        &[("ada@example.com", "USER", "ACTIVE")],
    );
    f.enable().await;

    let outcome = f.directory.sync_now(&f.owner).await.unwrap();
    assert_eq!(
        (outcome.groups, outcome.members, outcome.problem),
        (2, 4, None)
    );
    assert_eq!(f.members(staff).await, sorted(vec![ada, bob, cy]));
    assert_eq!(f.members(auditors).await, vec![ada]);
    assert_eq!(f.role(ada).await, "auditor");
    assert_eq!(f.role(bob).await, "member");

    // Removed in Google, removed here; the role that came with the group goes with it.
    f.google_has(
        "staff@example.com",
        &[("bob@example.com", "USER", "ACTIVE")],
    );
    f.google_has("auditors@example.com", &[]);
    f.directory.sync_now(&f.owner).await.unwrap();
    assert_eq!(f.members(staff).await, vec![bob]);
    assert!(f.members(auditors).await.is_empty());
    assert_eq!(f.role(ada).await, "member");

    let settings = f.directory.settings(&f.owner).await.unwrap();
    assert!(settings.enabled && settings.has_key && settings.can_store_key);
    assert_eq!(settings.client_email.as_deref(), Some(CLIENT));
    assert_eq!(settings.admin_email.as_deref(), Some("admin@example.com"));
    assert!(settings.synced_at_unix.is_some() && settings.problem.is_none());
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_group_google_cannot_find_is_named_while_others_sync() {
    let f = Fixture::new().await;
    let ada = f.person("ada", "ada@example.com").await;
    let gone = f.link("gone@example.com").await;
    let staff = f.link("staff@example.com").await;
    sqlx::query("INSERT INTO human_team_members (team_id, account_id) VALUES ($1, $2)")
        .bind(gone)
        .bind(ada)
        .execute(&f.pool)
        .await
        .unwrap();
    f.google_has(
        "staff@example.com",
        &[("ada@example.com", "USER", "ACTIVE")],
    );
    f.enable().await;

    let outcome = f.directory.sync_now(&f.owner).await.unwrap();
    assert_eq!((outcome.groups, outcome.members), (1, 1));
    let problem = outcome.problem.unwrap();
    assert!(
        problem.contains("gone@example.com") && problem.contains("404"),
        "{problem}"
    );
    assert_eq!(f.members(staff).await, vec![ada]);
    // A group that could not be read keeps who it had.
    assert_eq!(f.members(gone).await, vec![ada]);
    let settings = f.directory.settings(&f.owner).await.unwrap();
    assert_eq!(settings.problem.as_deref(), Some(problem.as_str()));

    let listed = f.directory.workspace_groups(&f.owner).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(
        (
            listed[0].email.as_str(),
            listed[0].name.as_str(),
            listed[0].direct_members_count
        ),
        ("staff@example.com", "staff", 1)
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn the_key_stays_sealed_and_only_administrators_change_the_directory() {
    let f = Fixture::new().await;
    let member = {
        f.identity
            .create_account_as(&f.owner, "mia", PASSWORD, Role::Member, Uuid::new_v4())
            .await
            .unwrap();
        sign_in(&f.identity, "mia").await
    };
    let auditor = {
        f.identity
            .create_account_as(&f.owner, "aud", PASSWORD, Role::Auditor, Uuid::new_v4())
            .await
            .unwrap();
        sign_in(&f.identity, "aud").await
    };

    // Nothing to sync with before a key and an administrator are saved.
    assert!(matches!(
        f.directory.sync_now(&f.owner).await,
        Err(IdentityError::Conflict)
    ));
    // Enabling needs a key; a key must be a service account's.
    assert!(matches!(
        f.directory.save(&f.owner, &save(true, None)).await,
        Err(IdentityError::InvalidInput)
    ));
    let mut wrong: Value = serde_json::from_str(&key_json()).unwrap();
    wrong["type"] = "authorized_user".into();
    assert!(
        f.directory
            .save(&f.owner, &save(true, Some(wrong.to_string())))
            .await
            .is_err()
    );
    for actor in [&member, &auditor] {
        assert!(matches!(
            f.directory.save(actor, &save(true, Some(key_json()))).await,
            Err(IdentityError::Forbidden)
        ));
        assert!(matches!(
            f.directory.sync_now(actor).await,
            Err(IdentityError::Forbidden)
        ));
        assert!(matches!(
            f.directory.workspace_groups(actor).await,
            Err(IdentityError::Forbidden)
        ));
    }
    assert!(matches!(
        f.directory.settings(&member).await,
        Err(IdentityError::Forbidden)
    ));

    f.enable().await;
    // Saving again without a key keeps the stored one.
    f.directory.save(&f.owner, &save(true, None)).await.unwrap();
    let settings = f.directory.settings(&auditor).await.unwrap();
    assert!(settings.has_key);
    let shown = serde_json::to_string(&settings).unwrap();
    let audited: Vec<String> = sqlx::query_scalar(
        "SELECT detail::text FROM audit_log UNION ALL SELECT input_body::text FROM identity_commands",
    )
    .fetch_all(&f.pool)
    .await
    .unwrap();
    assert!(audited.iter().any(|detail| detail.contains(CLIENT)));
    for text in audited.iter().chain([&shown]) {
        assert!(
            !text.contains("PRIVATE KEY") && !text.contains("private_key"),
            "{text}"
        );
    }
    let sealed: Vec<u8> = sqlx::query_scalar("SELECT service_account FROM google_directory")
        .fetch_one(&f.pool)
        .await
        .unwrap();
    assert!(!String::from_utf8_lossy(&sealed).contains("PRIVATE KEY"));

    // Without a credential key, no key can be stored.
    let unsealed = GoogleDirectory::new(f.pool.clone(), None);
    assert!(matches!(
        unsealed.save(&f.owner, &save(true, Some(key_json()))).await,
        Err(IdentityError::CredentialUnavailable(None))
    ));
    assert!(!unsealed.settings(&f.owner).await.unwrap().can_store_key);
}
