//! Members own what they create, share it within their own access, and manage their sign-in.
use crate::support;

use cubebot_orchestrator::{
    agents::{AgentId, Agents, CreateAgent},
    connections::Credentials,
    identity::{
        Actor, Identity, IdentityError, Role,
        access::{Capability, GrantRequest, Recipient, RevokeGrant, Scope},
        members::{
            ChangeRole, HandOver, Owned, Policy, Reactivate, SavePolicy, Transfer, TransferAll,
        },
        security::{AccountRequest, NewToken},
    },
    work::{CreateRoom, Work},
};
use uuid::Uuid;

const PASSWORD: &str = "long enough password";

struct People {
    identity: Identity,
    pool: sqlx::PgPool,
    owner: Actor,
}

impl People {
    async fn new() -> Self {
        let pool = support::database().await;
        let identity = Identity::new(pool.clone()).with_credentials(Credentials::new([7; 32]));
        identity.bootstrap("owner", PASSWORD).await.unwrap();
        let owner = sign_in(&identity, "owner").await;
        Self {
            identity,
            pool,
            owner,
        }
    }

    async fn add(&self, login: &str, role: Role) -> Actor {
        self.identity
            .create_account_as(&self.owner, login, PASSWORD, role, Uuid::new_v4())
            .await
            .unwrap();
        sign_in(&self.identity, login).await
    }
}

async fn sign_in(identity: &Identity, login: &str) -> Actor {
    let session = identity.login(login, PASSWORD).await.unwrap();
    identity.authenticate(&session.token).await.unwrap()
}

fn agent(name: &str) -> CreateAgent {
    CreateAgent {
        request_id: Uuid::new_v4(),
        name: name.into(),
        instructions: "Help".into(),
        avatar: None,
        voice: None,
    }
}

async fn allowed(identity: &Identity, actor: &Actor, scope: Scope, capability: Capability) -> bool {
    identity
        .explain_access(actor, scope, capability)
        .await
        .unwrap()
        .allowed
}

fn policy(agents: bool, limit: Option<i32>) -> SavePolicy {
    SavePolicy {
        request_id: Uuid::new_v4(),
        policy: Policy {
            members_create_agents: agents,
            members_create_rooms: true,
            members_create_teams: true,
            members_create_connections: true,
            members_bring_providers: true,
            member_agent_limit: limit,
        },
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn members_own_the_agents_they_create_within_the_policy() {
    let people = People::new().await;
    let (identity, agents) = (&people.identity, Agents::new(people.pool.clone()));
    let ada = people.add("ada", Role::Member).await;
    let bob = people.add("bob", Role::Member).await;
    let guest = people.add("gus", Role::Guest).await;
    let auditor = people.add("aud", Role::Auditor).await;

    let mine = agents.create(&ada, &agent("Ada's")).await.unwrap();
    let scope = Scope::Agent { id: mine.id };
    let chat = Scope::Conversation { id: mine.id };
    assert!(allowed(identity, &ada, scope, Capability::AgentManage).await);
    assert!(allowed(identity, &ada, chat, Capability::ConversationSend).await);
    let explained = identity
        .explain_access(&ada, scope, Capability::MemoryEdit)
        .await
        .unwrap();
    assert!(explained.owner && explained.grants.is_empty());
    assert!(!allowed(identity, &bob, scope, Capability::MemoryRead).await);
    assert!(agents.list(&bob, None).await.unwrap().is_empty());
    assert_eq!(agents.list(&ada, None).await.unwrap().len(), 1);

    // Auditors read everything and change nothing; guests create nothing.
    assert!(allowed(identity, &auditor, chat, Capability::ConversationRead).await);
    assert!(!allowed(identity, &auditor, chat, Capability::ConversationSend).await);
    assert!(!allowed(identity, &auditor, scope, Capability::AgentManage).await);
    for actor in [&guest, &auditor] {
        assert!(matches!(
            agents.create(actor, &agent("No")).await,
            Err(IdentityError::Forbidden)
        ));
    }

    // The default limit is three live agents per member.
    agents.create(&ada, &agent("Two")).await.unwrap();
    agents.create(&ada, &agent("Three")).await.unwrap();
    assert!(matches!(
        agents.create(&ada, &agent("Four")).await,
        Err(IdentityError::Forbidden)
    ));
    let standing = identity.standing(&ada).await.unwrap();
    assert!(!standing.can_create.agents && standing.owned_agents == 3);
    identity
        .save_policy(&people.owner, &policy(false, None))
        .await
        .unwrap();
    assert!(matches!(
        agents.create(&bob, &agent("Off")).await,
        Err(IdentityError::Forbidden)
    ));
    assert!(matches!(
        identity.save_policy(&ada, &policy(true, None)).await,
        Err(IdentityError::Forbidden)
    ));
    // Administrators are not held to the member policy.
    agents
        .create(&people.owner, &agent("Admin's"))
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn owners_share_no_more_than_they_hold() {
    let people = People::new().await;
    let (identity, agents) = (&people.identity, Agents::new(people.pool.clone()));
    let ada = people.add("ada", Role::Member).await;
    let bob = people.add("bob", Role::Member).await;
    let cy = people.add("cyd", Role::Member).await;
    let mine = agents.create(&ada, &agent("Shared")).await.unwrap();
    let grant = |actor: &Actor, to: Uuid, scope: Scope, capability: Capability| {
        let input = GrantRequest {
            request_id: Uuid::new_v4(),
            recipient: Recipient::User { id: to },
            scope,
            capability,
        };
        let identity = identity.clone();
        let actor = Actor {
            id: actor.id,
            administrator: actor.administrator,
            role: actor.role,
        };
        async move { identity.grant(&actor, &input).await }
    };
    let chat = Scope::Conversation { id: mine.id };
    let scope = Scope::Agent { id: mine.id };
    let shared = grant(&ada, bob.id, chat, Capability::ConversationSend)
        .await
        .unwrap();
    assert!(allowed(identity, &bob, chat, Capability::ConversationSend).await);
    assert_eq!(agents.list(&bob, None).await.unwrap().len(), 1);
    // Chatting is not managing: Bob cannot pass access on.
    assert!(matches!(
        grant(&bob, cy.id, chat, Capability::ConversationRead).await,
        Err(IdentityError::Forbidden)
    ));
    // A co-manager shares only what they hold.
    grant(&ada, bob.id, scope, Capability::AgentManage)
        .await
        .unwrap();
    assert!(matches!(
        grant(&bob, cy.id, scope, Capability::MemoryEdit).await,
        Err(IdentityError::Forbidden)
    ));
    grant(&bob, cy.id, chat, Capability::ConversationSend)
        .await
        .unwrap();
    let access = identity.resource_access(&ada, mine.id).await.unwrap();
    assert_eq!(access.owner, Some(ada.id));
    assert_eq!(access.grants.len(), 3);
    assert!(matches!(
        identity.resource_access(&cy, mine.id).await,
        Err(IdentityError::Forbidden)
    ));
    identity
        .revoke_grant(
            &ada,
            &RevokeGrant {
                request_id: Uuid::new_v4(),
                id: cubebot_orchestrator::identity::access::GrantId(shared.id),
                expected_revision: 1,
            },
        )
        .await
        .unwrap();
    assert!(!allowed(identity, &bob, chat, Capability::ConversationRead).await);
    // Guests see no directory to share from.
    let guest = people.add("gus", Role::Guest).await;
    assert!(matches!(
        identity.directory(&guest).await,
        Err(IdentityError::Forbidden)
    ));
    assert_eq!(identity.directory(&ada).await.unwrap().people.len(), 5);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn rooms_reach_only_agents_their_creator_may_use() {
    let people = People::new().await;
    let agents = Agents::new(people.pool.clone());
    let work = Work::new(people.pool.clone());
    let ada = people.add("ada", Role::Member).await;
    let bob = people.add("bob", Role::Member).await;
    let hers = agents.create(&ada, &agent("Hers")).await.unwrap();
    let his = agents.create(&bob, &agent("His")).await.unwrap();
    let room = |agents: Vec<Uuid>| CreateRoom {
        request_id: Uuid::new_v4(),
        name: "Plans".into(),
        agents,
    };
    assert!(matches!(
        work.create_room(&ada, &room(vec![hers.id, his.id])).await,
        Err(IdentityError::InvalidInput)
    ));
    let created = work.create_room(&ada, &room(vec![hers.id])).await.unwrap();
    assert_eq!(work.rooms(&ada).await.unwrap().len(), 1);
    assert!(work.rooms(&bob).await.unwrap().is_empty());
    assert!(matches!(
        work.delete_room(&bob, created.id).await,
        Err(IdentityError::NotFound)
    ));
    work.delete_room(&ada, created.id).await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn only_the_installation_owner_governs_administrators() {
    let people = People::new().await;
    let identity = &people.identity;
    let admin = people.add("adm", Role::Administrator).await;
    let ada = people.add("ada", Role::Member).await;
    let change = |id: Uuid, role: Role, revision: i64| ChangeRole {
        request_id: Uuid::new_v4(),
        id,
        role,
        expected_revision: revision,
    };
    assert!(matches!(
        identity
            .change_role(&admin, &change(ada.id, Role::Administrator, 1))
            .await,
        Err(IdentityError::Forbidden)
    ));
    identity
        .change_role(&admin, &change(ada.id, Role::Auditor, 1))
        .await
        .unwrap();
    assert!(matches!(
        identity
            .change_role(&admin, &change(people.owner.id, Role::Member, 1))
            .await,
        Err(IdentityError::InvalidInput)
    ));
    assert!(matches!(
        identity
            .suspend(&admin, people.owner.id, 1, Uuid::new_v4())
            .await,
        Err(IdentityError::InvalidInput)
    ));
    let receipt = identity
        .suspend(&people.owner, admin.id, 1, Uuid::new_v4())
        .await
        .unwrap();
    assert!(identity.login("adm", PASSWORD).await.is_err());
    identity
        .reactivate(
            &people.owner,
            &Reactivate {
                request_id: Uuid::new_v4(),
                id: admin.id,
                expected_revision: receipt.revision,
            },
        )
        .await
        .unwrap();
    let admin = sign_in(identity, "adm").await;
    identity
        .hand_over(
            &people.owner,
            &HandOver {
                request_id: Uuid::new_v4(),
                to: admin.id,
            },
        )
        .await
        .unwrap();
    assert!(identity.standing(&admin).await.unwrap().owner);
    assert!(!identity.standing(&people.owner).await.unwrap().owner);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn ownership_moves_with_a_transfer() {
    let people = People::new().await;
    let (identity, agents) = (&people.identity, Agents::new(people.pool.clone()));
    let ada = people.add("ada", Role::Member).await;
    let bob = people.add("bob", Role::Member).await;
    let first = agents.create(&ada, &agent("First")).await.unwrap();
    let second = agents.create(&ada, &agent("Second")).await.unwrap();
    let transfer = |actor: &Actor, id: Uuid| {
        let input = Transfer {
            request_id: Uuid::new_v4(),
            kind: Owned::Agent,
            id,
            to: bob.id,
        };
        let identity = identity.clone();
        let actor = Actor {
            id: actor.id,
            administrator: actor.administrator,
            role: actor.role,
        };
        async move { identity.transfer(&actor, &input).await }
    };
    assert!(matches!(
        transfer(&bob, first.id).await,
        Err(IdentityError::NotFound)
    ));
    transfer(&ada, first.id).await.unwrap();
    let scope = Scope::Agent { id: first.id };
    assert!(allowed(identity, &bob, scope, Capability::AgentManage).await);
    assert!(!allowed(identity, &ada, scope, Capability::AgentManage).await);
    identity
        .transfer_all(
            &people.owner,
            &TransferAll {
                request_id: Uuid::new_v4(),
                from: ada.id,
                to: bob.id,
            },
        )
        .await
        .unwrap();
    assert!(agents.get(&bob, AgentId(second.id)).await.is_ok());
    assert!(agents.list(&ada, None).await.unwrap().is_empty());
}

fn base32_decode(text: &str) -> Vec<u8> {
    let (mut buffer, mut bits, mut out) = (0u32, 0u32, Vec::new());
    for byte in text.bytes() {
        let value = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567"
            .iter()
            .position(|letter| *letter == byte)
            .unwrap() as u32;
        buffer = (buffer << 5) | value;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            out.push((buffer >> bits) as u8);
        }
    }
    out
}

fn code(secret: &[u8], offset: i64) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let counter = (now / 30 + offset) as u64;
    let key = ring::hmac::Key::new(ring::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, secret);
    let tag = ring::hmac::sign(&key, &counter.to_be_bytes());
    let tag = tag.as_ref();
    let at = usize::from(tag[19] & 0x0f);
    let value = u32::from_be_bytes([tag[at], tag[at + 1], tag[at + 2], tag[at + 3]]);
    format!("{:06}", (value & 0x7fff_ffff) % 1_000_000)
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn two_factor_codes_and_recovery_codes_work_once() {
    let people = People::new().await;
    let identity = &people.identity;
    let ada = people.add("ada", Role::Member).await;
    let enrollment = identity.enroll_two_factor(&ada).await.unwrap();
    let secret = base32_decode(&enrollment.secret);
    assert!(enrollment.uri.starts_with("otpauth://totp/Cubebot:ada?"));
    assert!(matches!(
        identity.confirm_two_factor(&ada, "000000x").await,
        Err(IdentityError::InvalidInput)
    ));
    let recovery = identity
        .confirm_two_factor(&ada, &code(&secret, -1))
        .await
        .unwrap();
    assert_eq!(recovery.len(), 10);
    assert!(matches!(
        identity.login("ada", PASSWORD).await,
        Err(IdentityError::TwoFactorRequired)
    ));
    let current = code(&secret, 0);
    identity
        .login_with("ada", PASSWORD, Some(&current))
        .await
        .unwrap();
    // The same code does not work twice.
    assert!(matches!(
        identity.login_with("ada", PASSWORD, Some(&current)).await,
        Err(IdentityError::Unauthorized)
    ));
    identity
        .login_with("ada", PASSWORD, Some(&recovery[0].to_uppercase()))
        .await
        .unwrap();
    assert!(matches!(
        identity
            .login_with("ada", PASSWORD, Some(&recovery[0]))
            .await,
        Err(IdentityError::Unauthorized)
    ));
    let listed = identity.accounts(&people.owner, None).await.unwrap();
    assert!(
        listed
            .iter()
            .any(|account| account.login == "ada" && account.two_factor)
    );
    identity
        .reset_two_factor(
            &people.owner,
            &AccountRequest {
                request_id: Uuid::new_v4(),
                id: ada.id,
            },
        )
        .await
        .unwrap();
    identity.login("ada", PASSWORD).await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn people_manage_their_password_sessions_and_tokens() {
    let people = People::new().await;
    let identity = &people.identity;
    let ada = people.add("ada", Role::Member).await;
    let kept = identity.login("ada", PASSWORD).await.unwrap();
    let other = identity.login("ada", PASSWORD).await.unwrap();
    let token = identity
        .create_token(
            &ada,
            &NewToken {
                name: "CI".into(),
                days: 30,
            },
            &kept.token,
        )
        .await
        .unwrap();
    assert_eq!(identity.authenticate(&token).await.unwrap().id, ada.id);
    // A token made from a single sign-on session ends with that sign-on policy.
    let digest = |token: &str| {
        use sha2::Digest;
        sha2::Sha256::digest(token.as_bytes()).to_vec()
    };
    sqlx::query!(
        "UPDATE human_sessions SET oidc_policy_revision = 42 WHERE token_hash = $1",
        digest(&other.token)
    )
    .execute(&people.pool)
    .await
    .unwrap();
    let bound = identity
        .create_token(
            &ada,
            &NewToken {
                name: "SSO".into(),
                days: 1,
            },
            &other.token,
        )
        .await
        .unwrap();
    assert!(identity.authenticate(&bound).await.is_err());
    sqlx::query!(
        "UPDATE human_sessions SET oidc_policy_revision = NULL WHERE token_hash = $1",
        digest(&other.token)
    )
    .execute(&people.pool)
    .await
    .unwrap();
    let sessions = identity.sessions(&ada, &kept.token).await.unwrap();
    assert_eq!(sessions.iter().filter(|session| session.current).count(), 1);
    let named = sessions
        .iter()
        .find(|session| session.name.as_deref() == Some("CI"))
        .unwrap();
    identity.end_session(&ada, named.id).await.unwrap();
    assert!(identity.authenticate(&token).await.is_err());
    assert!(matches!(
        identity
            .change_password(&ada, "wrong password!!", "a new long password", &kept.token)
            .await,
        Err(IdentityError::InvalidInput)
    ));
    identity
        .change_password(&ada, PASSWORD, "a new long password", &kept.token)
        .await
        .unwrap();
    assert!(identity.authenticate(&kept.token).await.is_ok());
    assert!(identity.authenticate(&other.token).await.is_err());
    identity.login("ada", "a new long password").await.unwrap();

    let reset = identity
        .create_password_reset(
            &people.owner,
            &AccountRequest {
                request_id: Uuid::new_v4(),
                id: ada.id,
            },
        )
        .await
        .unwrap();
    identity
        .redeem_password_reset(&reset, "reset long password")
        .await
        .unwrap();
    assert!(identity.authenticate(&kept.token).await.is_err());
    assert!(
        identity
            .redeem_password_reset(&reset, "again long password")
            .await
            .is_err()
    );
    identity.login("ada", "reset long password").await.unwrap();
}

async fn revision(pool: &sqlx::PgPool, id: Uuid) -> i64 {
    sqlx::query_scalar!("SELECT revision FROM human_accounts WHERE id = $1", id)
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn set_role(people: &People, id: Uuid, role: Role) {
    let expected_revision = revision(&people.pool, id).await;
    people
        .identity
        .change_role(
            &people.owner,
            &ChangeRole {
                request_id: Uuid::new_v4(),
                id,
                role,
                expected_revision,
            },
        )
        .await
        .unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn auditors_and_guests_hold_nothing_on_what_they_once_created() {
    let people = People::new().await;
    let (identity, agents) = (&people.identity, Agents::new(people.pool.clone()));
    let ada = people.add("ada", Role::Member).await;
    let gus = people.add("gus", Role::Member).await;
    let hers = agents.create(&ada, &agent("Hers")).await.unwrap();
    let his = agents.create(&gus, &agent("His")).await.unwrap();
    set_role(&people, ada.id, Role::Auditor).await;
    set_role(&people, gus.id, Role::Guest).await;
    let ada = sign_in(identity, "ada").await;
    let gus = sign_in(identity, "gus").await;
    let scope = Scope::Agent { id: hers.id };
    assert!(!allowed(identity, &ada, scope, Capability::AgentManage).await);
    assert!(!allowed(identity, &ada, scope, Capability::MemoryEdit).await);
    assert!(allowed(identity, &ada, scope, Capability::MemoryRead).await);
    let chat = Scope::Conversation { id: his.id };
    assert!(!allowed(identity, &gus, chat, Capability::ConversationRead).await);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn the_agent_limit_holds_across_transfers_and_retries() {
    let people = People::new().await;
    let (identity, agents) = (&people.identity, Agents::new(people.pool.clone()));
    identity
        .save_policy(&people.owner, &policy(true, Some(1)))
        .await
        .unwrap();
    let ada = people.add("ada", Role::Member).await;
    let bob = people.add("bob", Role::Member).await;
    let command = agent("Only");
    let first = agents.create(&ada, &command).await.unwrap();
    // A retry after the limit is reached still gets its receipt.
    assert_eq!(agents.create(&ada, &command).await.unwrap().id, first.id);
    agents.create(&bob, &agent("Bob's")).await.unwrap();
    let hand = Transfer {
        request_id: Uuid::new_v4(),
        kind: Owned::Agent,
        id: first.id,
        to: bob.id,
    };
    assert!(matches!(
        identity.transfer(&ada, &hand).await,
        Err(IdentityError::Forbidden)
    ));
    // Administrators move agents regardless, such as when someone leaves.
    identity.transfer(&people.owner, &hand).await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn reset_links_lapse_with_their_makers_authority() {
    let people = People::new().await;
    let identity = &people.identity;
    let admin = people.add("adm", Role::Administrator).await;
    let ada = people.add("ada", Role::Member).await;
    let bob = people.add("bob", Role::Member).await;
    let link = |actor: &Actor, id: Uuid| {
        let identity = identity.clone();
        let actor = Actor {
            id: actor.id,
            administrator: actor.administrator,
            role: actor.role,
        };
        async move {
            identity
                .create_password_reset(
                    &actor,
                    &AccountRequest {
                        request_id: Uuid::new_v4(),
                        id,
                    },
                )
                .await
                .unwrap()
        }
    };
    let for_ada = link(&admin, ada.id).await;
    let for_bob = link(&admin, bob.id).await;
    // Ada became an administrator: only the owner's links reset administrators.
    set_role(&people, ada.id, Role::Administrator).await;
    assert!(matches!(
        identity
            .redeem_password_reset(&for_ada, "a brand new password")
            .await,
        Err(IdentityError::Unauthorized)
    ));
    // Its maker was suspended.
    identity
        .suspend(
            &people.owner,
            admin.id,
            revision(&people.pool, admin.id).await,
            Uuid::new_v4(),
        )
        .await
        .unwrap();
    assert!(matches!(
        identity
            .redeem_password_reset(&for_bob, "a brand new password")
            .await,
        Err(IdentityError::Unauthorized)
    ));
    let owners = link(&people.owner, ada.id).await;
    identity
        .redeem_password_reset(&owners, "a brand new password")
        .await
        .unwrap();
    identity.login("ada", "a brand new password").await.unwrap();
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn guessing_two_factor_codes_spends_the_sign_in_budget() {
    let people = People::new().await;
    let identity = &people.identity;
    let ada = people.add("ada", Role::Member).await;
    identity.enroll_two_factor(&ada).await.unwrap();
    let mut refused = 0;
    for _ in 0..12 {
        match identity.confirm_two_factor(&ada, "000000").await {
            Err(IdentityError::InvalidInput) => {}
            Err(IdentityError::Unauthorized) => refused += 1,
            other => panic!("unexpected {:?}", other.map(|_| ())),
        }
    }
    assert!(refused >= 2, "attempts past the budget are refused");
}
