use crate::support;

use cubebot_orchestrator::identity::{Identity, IdentityError};

async fn administrator(identity: &Identity) -> cubebot_orchestrator::identity::Actor {
    identity
        .bootstrap("invite-admin", "long invitation administrator password")
        .await
        .unwrap();
    let session = identity
        .login("invite-admin", "long invitation administrator password")
        .await
        .unwrap();
    identity.authenticate(&session.token).await.unwrap()
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn invitations_admit_the_bound_login_exactly_once() {
    let identity = Identity::new(support::database().await);
    let admin = administrator(&identity).await;
    let invite = identity
        .invite(&admin, "invited-member", uuid::Uuid::new_v4())
        .await
        .unwrap();
    assert!(matches!(
        identity
            .accept_invitation("invalid", "long invitation password", uuid::Uuid::new_v4())
            .await,
        Err(IdentityError::Unauthorized)
    ));
    let (first, second) = tokio::join!(
        identity.accept_invitation(
            invite.token.as_deref().unwrap(),
            "long invitation password",
            uuid::Uuid::new_v4()
        ),
        identity.accept_invitation(
            invite.token.as_deref().unwrap(),
            "another invitation password",
            uuid::Uuid::new_v4()
        )
    );
    assert_ne!(first.is_ok(), second.is_ok());
    let password = if first.is_ok() {
        "long invitation password"
    } else {
        "another invitation password"
    };
    let session = identity.login("invited-member", password).await.unwrap();
    let member = identity.authenticate(&session.token).await.unwrap();
    assert!(!member.administrator);
    assert!(matches!(
        identity
            .invite(&member, "escalated-member", uuid::Uuid::new_v4())
            .await,
        Err(IdentityError::Forbidden)
    ));
    identity
        .suspend(&admin, member.id, 1, uuid::Uuid::new_v4())
        .await
        .unwrap();
    let replacement = identity
        .invite(&admin, "invited-member", uuid::Uuid::new_v4())
        .await;
    assert!(matches!(replacement, Err(IdentityError::Conflict)));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn revoked_and_expired_invitations_cannot_be_redeemed() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    let admin = administrator(&identity).await;
    let revoked = identity
        .invite(&admin, "revoked-member", uuid::Uuid::new_v4())
        .await
        .unwrap();
    identity
        .revoke_invitation(&admin, revoked.id, 1, uuid::Uuid::new_v4())
        .await
        .unwrap();
    assert!(matches!(
        identity
            .accept_invitation(
                revoked.token.as_deref().unwrap(),
                "long member password",
                uuid::Uuid::new_v4()
            )
            .await,
        Err(IdentityError::Unauthorized)
    ));
    let expired = identity
        .invite(&admin, "expired-member", uuid::Uuid::new_v4())
        .await
        .unwrap();
    sqlx::query("UPDATE invitations SET expires_at = now() - interval '1 second' WHERE id = $1")
        .bind(expired.id)
        .execute(&pool)
        .await
        .unwrap();
    assert!(matches!(
        identity
            .accept_invitation(
                expired.token.as_deref().unwrap(),
                "long member password",
                uuid::Uuid::new_v4()
            )
            .await,
        Err(IdentityError::Unauthorized)
    ));
}

async fn await_blocked_redemption(pool: &sqlx::PgPool) {
    for _ in 0..150 {
        let waiting: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE wait_event_type = 'Lock' AND query LIKE 'SELECT id FROM invitations%FOR UPDATE')")
            .fetch_one(pool).await.unwrap();
        if waiting {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("redemption did not reach its row lock");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn invitation_expiring_while_redemption_waits_creates_no_account() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    let admin = administrator(&identity).await;
    let invitation = identity
        .invite(&admin, "waiting-member", uuid::Uuid::new_v4())
        .await
        .unwrap();
    let mut blocker = pool.begin().await.unwrap();
    sqlx::query("SELECT id FROM invitations WHERE id = $1 FOR UPDATE")
        .bind(invitation.id)
        .execute(&mut *blocker)
        .await
        .unwrap();
    let redemption = tokio::spawn(async move {
        identity
            .accept_invitation(
                invitation.token.as_deref().unwrap(),
                "long invited password",
                uuid::Uuid::new_v4(),
            )
            .await
    });
    await_blocked_redemption(&pool).await;
    // Expiry occurs after the waiting transaction began, before it acquires the lock.
    sqlx::query("UPDATE invitations SET expires_at = clock_timestamp() + interval '100 milliseconds' WHERE id = $1")
        .bind(invitation.id).execute(&mut *blocker).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    blocker.commit().await.unwrap();
    assert!(matches!(
        redemption.await.unwrap(),
        Err(IdentityError::Unauthorized)
    ));
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM human_accounts WHERE login = 'waiting-member'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 0);
}
