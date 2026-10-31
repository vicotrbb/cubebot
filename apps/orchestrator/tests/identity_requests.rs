use crate::support;

use cubebot_orchestrator::identity::{Identity, IdentityError};
use uuid::Uuid;

async fn admin(identity: &Identity) -> cubebot_orchestrator::identity::Actor {
    identity
        .bootstrap("request-admin", "long administrator password")
        .await
        .unwrap();
    let session = identity
        .login("request-admin", "long administrator password")
        .await
        .unwrap();
    identity.authenticate(&session.token).await.unwrap()
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn account_creation_replays_without_journaling_passwords() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    let actor = admin(&identity).await;
    let request = Uuid::new_v4();
    let (one, two) = tokio::join!(
        identity.create_account(
            &actor,
            "retry-member",
            "long member secret password",
            false,
            request
        ),
        identity.create_account(
            &actor,
            "retry-member",
            "long member secret password",
            false,
            request
        )
    );
    let id = one.unwrap();
    assert_eq!(id, two.unwrap());
    assert!(matches!(
        identity
            .create_account(
                &actor,
                "retry-member",
                "changed member secret password",
                false,
                request
            )
            .await,
        Err(IdentityError::Conflict)
    ));
    assert!(matches!(
        identity
            .create_account(
                &actor,
                "retry-member",
                "long member secret password",
                true,
                request
            )
            .await,
        Err(IdentityError::Conflict)
    ));
    let identity = Identity::new(
        sqlx::PgPool::connect_with(pool.connect_options().as_ref().clone())
            .await
            .unwrap(),
    );
    assert_eq!(
        id,
        identity
            .create_account(
                &actor,
                "retry-member",
                "long member secret password",
                false,
                request
            )
            .await
            .unwrap()
    );
    let payload: (String, String) = sqlx::query!(
        r#"
        SELECT input_body::text AS "input_body!", secret_verifier AS "secret_verifier!"
        FROM identity_commands
        WHERE kind = 'account.create'
        "#
    )
    .fetch_one(&pool)
    .await
    .map(|row| (row.input_body, row.secret_verifier))
    .unwrap();
    assert!(!payload.0.contains("password"));
    assert!(payload.1.starts_with("$argon2id$"));
    let audit = serde_json::to_string(&identity.access_events(&actor, 0).await.unwrap()).unwrap();
    assert!(!audit.contains("long member secret password"));
    assert!(!audit.contains("$argon2"));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn invitation_creation_and_redemption_return_their_original_outcome() {
    let identity = Identity::new(support::database().await);
    let actor = admin(&identity).await;
    let request = Uuid::new_v4();
    let issued = identity
        .invite(&actor, "invitation-member", request)
        .await
        .unwrap();
    let repeated = identity
        .invite(&actor, "invitation-member", request)
        .await
        .unwrap();
    assert_eq!(issued.id, repeated.id);
    assert!(repeated.token.is_none());
    let token = issued.token.unwrap();
    let accept = Uuid::new_v4();
    let id = identity
        .accept_invitation(&token, "long invited password", accept)
        .await
        .unwrap();
    assert_eq!(
        id,
        identity
            .accept_invitation(&token, "long invited password", accept)
            .await
            .unwrap()
    );
    assert!(matches!(
        identity
            .accept_invitation(&token, "changed invited password", accept)
            .await,
        Err(IdentityError::Conflict)
    ));
    assert!(matches!(
        identity
            .accept_invitation(&token, "long invited password", Uuid::new_v4())
            .await,
        Err(IdentityError::Unauthorized)
    ));
    let suspension = Uuid::new_v4();
    let result = identity.suspend(&actor, id, 1, suspension).await.unwrap();
    assert_eq!(
        result,
        identity.suspend(&actor, id, 1, suspension).await.unwrap()
    );
    assert!(matches!(
        identity
            .accept_invitation(&token, "long invited password", accept)
            .await,
        Err(IdentityError::Unauthorized)
    ));
    assert!(
        !serde_json::to_string(&identity.access_events(&actor, 0).await.unwrap())
            .unwrap()
            .contains(&token)
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn revoked_invitation_commands_replay_but_cannot_reactivate_the_token() {
    let identity = Identity::new(support::database().await);
    let actor = admin(&identity).await;
    let invite_request = Uuid::new_v4();
    let issued = identity
        .invite(&actor, "revoked-retry", invite_request)
        .await
        .unwrap();
    let request = Uuid::new_v4();
    let result = identity
        .revoke_invitation(&actor, issued.id, 1, request)
        .await
        .unwrap();
    assert_eq!(
        result,
        identity
            .revoke_invitation(&actor, issued.id, 1, request)
            .await
            .unwrap()
    );
    assert!(matches!(
        identity
            .revoke_invitation(&actor, issued.id, 1, Uuid::new_v4())
            .await,
        Err(IdentityError::Conflict)
    ));
    assert!(
        identity
            .invite(&actor, "revoked-retry", invite_request)
            .await
            .unwrap()
            .token
            .is_none()
    );
    assert!(matches!(
        identity
            .accept_invitation(
                &issued.token.unwrap(),
                "long revoked password",
                Uuid::new_v4()
            )
            .await,
        Err(IdentityError::Unauthorized)
    ));
    let listing = identity.invitations(&actor, None).await.unwrap();
    assert_eq!(listing[0].status, "revoked");
    assert_eq!(listing[0].revision, 2);
    let accounts = identity.accounts(&actor, None).await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert!(accounts[0].active);
}
