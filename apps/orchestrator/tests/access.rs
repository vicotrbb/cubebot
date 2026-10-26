mod support;

use cubebot_orchestrator::identity::{
    Identity, IdentityError,
    access::{Capability, GrantRequest, Recipient, Scope},
    teams::{CreateTeam, MembershipChange, TeamId},
};
use uuid::Uuid;

async fn people(
    identity: &Identity,
) -> (
    cubebot_orchestrator::identity::Actor,
    cubebot_orchestrator::identity::Actor,
) {
    identity
        .bootstrap("access-admin", "long administrator password")
        .await
        .unwrap();
    let admin = identity
        .login("access-admin", "long administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&admin.token).await.unwrap();
    identity
        .create_account(
            &admin,
            "access-member",
            "long member password",
            false,
            uuid::Uuid::new_v4(),
        )
        .await
        .unwrap();
    let member = identity
        .login("access-member", "long member password")
        .await
        .unwrap();
    (admin, identity.authenticate(&member.token).await.unwrap())
}

fn membership(
    team: TeamId,
    account_id: Uuid,
    expected_revision: i64,
    present: bool,
) -> MembershipChange {
    MembershipChange {
        request_id: Uuid::new_v4(),
        team,
        account_id,
        expected_revision,
        present,
    }
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn team_access_is_scoped_explainable_and_revoked_without_relogin() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    let (admin, member) = people(&identity).await;
    let team = identity
        .create_team(
            &admin,
            &CreateTeam {
                request_id: Uuid::new_v4(),
                name: "Engineering".into(),
            },
        )
        .await
        .unwrap();
    let team_id = TeamId(team.id);
    identity
        .change_membership(&admin, &membership(team_id, member.id, team.revision, true))
        .await
        .unwrap();
    let scope = Scope::Agent { id: Uuid::new_v4() };
    let grant = identity
        .grant(
            &admin,
            &GrantRequest {
                request_id: Uuid::new_v4(),
                recipient: Recipient::Team { id: team_id },
                scope,
                capability: Capability::MemoryRead,
            },
        )
        .await
        .unwrap();
    let explanation = identity
        .explain_access(&member, scope, Capability::MemoryRead)
        .await
        .unwrap();
    assert!(explanation.allowed);
    assert_eq!(explanation.grants[0].team_id, Some(team_id));
    assert!(
        !identity
            .explain_access(&member, scope, Capability::MemoryEdit)
            .await
            .unwrap()
            .allowed
    );
    assert!(
        !identity
            .explain_access(
                &member,
                Scope::Agent { id: Uuid::new_v4() },
                Capability::MemoryRead
            )
            .await
            .unwrap()
            .allowed
    );
    identity
        .change_membership(&admin, &membership(team_id, member.id, 2, false))
        .await
        .unwrap();
    assert!(
        !identity
            .explain_access(&member, scope, Capability::MemoryRead)
            .await
            .unwrap()
            .allowed
    );
    assert!(
        identity
            .explain_access(&admin, scope, Capability::MemoryRead)
            .await
            .unwrap()
            .administrator
    );
    let retained: i64 = sqlx::query_scalar("SELECT count(*) FROM access_grants WHERE id = $1")
        .bind(grant.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(retained, 1);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn team_commands_replay_after_reconnect_and_reject_changed_input_or_stale_revision() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    let (admin, member) = people(&identity).await;
    let command = CreateTeam {
        request_id: Uuid::new_v4(),
        name: "Research".into(),
    };
    let (first, second) = tokio::join!(
        identity.create_team(&admin, &command),
        identity.create_team(&admin, &command)
    );
    let team = first.unwrap();
    assert_eq!(team, second.unwrap());
    let identity = Identity::new(
        sqlx::PgPool::connect_with(pool.connect_options().as_ref().clone())
            .await
            .unwrap(),
    );
    assert_eq!(team, identity.create_team(&admin, &command).await.unwrap());
    let changed = CreateTeam {
        name: "Changed".into(),
        ..command
    };
    assert!(matches!(
        identity.create_team(&admin, &changed).await,
        Err(IdentityError::Conflict)
    ));
    let add = membership(TeamId(team.id), member.id, 1, true);
    let added = identity.change_membership(&admin, &add).await.unwrap();
    assert_eq!(
        added,
        identity.change_membership(&admin, &add).await.unwrap()
    );
    assert!(matches!(
        identity
            .change_membership(&admin, &membership(TeamId(team.id), member.id, 1, false))
            .await,
        Err(IdentityError::Conflict)
    ));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM identity_events WHERE kind = 'team.membership'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn manager_permissions_never_confer_administration_or_content_and_suspension_wins() {
    let identity = Identity::new(support::database().await);
    let (admin, member) = people(&identity).await;
    let scope = Scope::Agent { id: Uuid::new_v4() };
    let command = GrantRequest {
        request_id: Uuid::new_v4(),
        recipient: Recipient::User { id: member.id },
        scope,
        capability: Capability::AgentManage,
    };
    identity.grant(&admin, &command).await.unwrap();
    assert!(
        identity
            .explain_access(&member, scope, Capability::AgentManage)
            .await
            .unwrap()
            .allowed
    );
    assert!(
        !identity
            .explain_access(&member, scope, Capability::MemoryRead)
            .await
            .unwrap()
            .allowed
    );
    assert!(matches!(
        identity.grant(&member, &command).await,
        Err(IdentityError::Forbidden)
    ));
    identity
        .suspend(&admin, member.id, 1, uuid::Uuid::new_v4())
        .await
        .unwrap();
    assert!(matches!(
        identity
            .explain_access(&member, scope, Capability::AgentManage)
            .await,
        Err(IdentityError::Unauthorized)
    ));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn revoking_a_direct_grant_preserves_team_access_until_team_deletion() {
    use cubebot_orchestrator::identity::{
        access::{GrantId, RevokeGrant},
        teams::DeleteTeam,
    };
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    let (admin, member) = people(&identity).await;
    let team = identity
        .create_team(
            &admin,
            &CreateTeam {
                request_id: Uuid::new_v4(),
                name: "Shared".into(),
            },
        )
        .await
        .unwrap();
    identity
        .change_membership(&admin, &membership(TeamId(team.id), member.id, 1, true))
        .await
        .unwrap();
    let scope = Scope::Agent { id: Uuid::new_v4() };
    let direct = identity
        .grant(
            &admin,
            &GrantRequest {
                request_id: Uuid::new_v4(),
                recipient: Recipient::User { id: member.id },
                scope,
                capability: Capability::BrowserView,
            },
        )
        .await
        .unwrap();
    identity
        .grant(
            &admin,
            &GrantRequest {
                request_id: Uuid::new_v4(),
                recipient: Recipient::Team {
                    id: TeamId(team.id),
                },
                scope,
                capability: Capability::BrowserView,
            },
        )
        .await
        .unwrap();
    assert_eq!(
        identity
            .explain_access(&member, scope, Capability::BrowserView)
            .await
            .unwrap()
            .grants
            .len(),
        2
    );
    assert!(
        !identity
            .explain_access(&member, scope, Capability::BrowserControl)
            .await
            .unwrap()
            .allowed
    );
    let revoke = RevokeGrant {
        request_id: Uuid::new_v4(),
        id: GrantId(direct.id),
        expected_revision: 1,
    };
    let result = identity.revoke_grant(&admin, &revoke).await.unwrap();
    assert_eq!(
        result,
        identity.revoke_grant(&admin, &revoke).await.unwrap()
    );
    let explanation = identity
        .explain_access(&member, scope, Capability::BrowserView)
        .await
        .unwrap();
    assert!(explanation.allowed);
    assert_eq!(explanation.grants.len(), 1);
    identity
        .delete_team(
            &admin,
            &DeleteTeam {
                request_id: Uuid::new_v4(),
                team: TeamId(team.id),
                expected_revision: 2,
            },
        )
        .await
        .unwrap();
    assert!(
        !identity
            .explain_access(&member, scope, Capability::BrowserView)
            .await
            .unwrap()
            .allowed
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM access_grants")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn replay_and_receipt_reads_recheck_current_administrator_authority() {
    let identity = Identity::new(support::database().await);
    let (admin, _) = people(&identity).await;
    let command = CreateTeam {
        request_id: Uuid::new_v4(),
        name: "Private".into(),
    };
    identity.create_team(&admin, &command).await.unwrap();
    identity
        .create_account(
            &admin,
            "second-admin",
            "second administrator password",
            true,
            uuid::Uuid::new_v4(),
        )
        .await
        .unwrap();
    let session = identity
        .login("second-admin", "second administrator password")
        .await
        .unwrap();
    let second = identity.authenticate(&session.token).await.unwrap();
    identity
        .suspend(&second, admin.id, 1, uuid::Uuid::new_v4())
        .await
        .unwrap();
    assert!(matches!(
        identity.create_team(&admin, &command).await,
        Err(IdentityError::Forbidden)
    ));
    assert!(matches!(
        identity
            .command_receipt(&admin, "team.create", command.request_id)
            .await,
        Err(IdentityError::Forbidden)
    ));
    assert!(matches!(
        identity
            .command_receipt(&second, "team.create", command.request_id)
            .await,
        Err(IdentityError::NotFound)
    ));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn team_listing_pages_and_denies_non_administrators() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    let (admin, member) = people(&identity).await;
    sqlx::query(
        "INSERT INTO human_teams (name) SELECT 'Page ' || n FROM generate_series(1, 105) n",
    )
    .execute(&pool)
    .await
    .unwrap();
    let page = identity.teams(&admin, None).await.unwrap();
    assert_eq!(page.len(), 100);
    let tail = identity.teams(&admin, Some(page[99].id)).await.unwrap();
    assert_eq!(tail.len(), 5);
    assert!(matches!(
        identity.teams(&member, None).await,
        Err(IdentityError::Forbidden)
    ));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn publication_barrier_prevents_cursor_gaps_between_administrators() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    let (admin, _) = people(&identity).await;
    identity
        .create_account(
            &admin,
            "event-admin",
            "long event administrator password",
            true,
            uuid::Uuid::new_v4(),
        )
        .await
        .unwrap();
    let session = identity
        .login("event-admin", "long event administrator password")
        .await
        .unwrap();
    let second = identity.authenticate(&session.token).await.unwrap();
    let cursor = identity
        .access_events(&admin, 0)
        .await
        .unwrap()
        .last()
        .unwrap()
        .sequence;
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT singleton FROM installation WHERE singleton FOR UPDATE")
        .execute(&mut *blocker)
        .await
        .unwrap();
    let other = identity.clone();
    let actor_id = admin.id;
    let mut first = tokio::spawn(async move {
        other
            .create_team(
                &cubebot_orchestrator::identity::Actor {
                    id: actor_id,
                    administrator: true,
                },
                &CreateTeam {
                    request_id: Uuid::new_v4(),
                    name: "First".into(),
                },
            )
            .await
    });
    let other = identity.clone();
    let later = tokio::spawn(async move {
        other
            .create_team(
                &second,
                &CreateTeam {
                    request_id: Uuid::new_v4(),
                    name: "Second".into(),
                },
            )
            .await
    });
    let observed = tokio::time::timeout(std::time::Duration::from_millis(200), &mut first).await;
    assert!(
        observed.is_err(),
        "publication must wait for the shared commit-order barrier"
    );
    assert!(
        identity
            .access_events(&admin, cursor)
            .await
            .unwrap()
            .is_empty()
    );
    blocker.commit().await.unwrap();
    first.await.unwrap().unwrap();
    later.await.unwrap().unwrap();
    let events = identity.access_events(&admin, cursor).await.unwrap();
    assert_eq!(events.len(), 2);
    let tail = identity
        .access_events(&admin, events[0].sequence)
        .await
        .unwrap();
    assert_eq!(tail.len(), 1);
    assert_eq!(tail[0].sequence, events[1].sequence);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn membership_cleanup_accepts_suspended_accounts() {
    let identity = Identity::new(support::database().await);
    let (admin, member) = people(&identity).await;
    let team = identity
        .create_team(
            &admin,
            &CreateTeam {
                request_id: Uuid::new_v4(),
                name: "Cleanup".into(),
            },
        )
        .await
        .unwrap();
    let id = TeamId(team.id);
    identity
        .change_membership(&admin, &membership(id, member.id, 1, true))
        .await
        .unwrap();
    identity
        .suspend(&admin, member.id, 1, uuid::Uuid::new_v4())
        .await
        .unwrap();
    identity
        .change_membership(&admin, &membership(id, member.id, 2, false))
        .await
        .unwrap();
}
