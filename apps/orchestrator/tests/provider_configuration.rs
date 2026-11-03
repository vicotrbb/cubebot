use crate::support;
use cubebot_orchestrator::{
    connections::{ConfigureProvider, Connections, Credentials},
    identity::{Identity, IdentityError},
};
use uuid::Uuid;

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn provider_is_encrypted_durable_and_administered_with_revision_checks() {
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
    identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let session = identity.login("member", "member password").await.unwrap();
    let member = identity.authenticate(&session.token).await.unwrap();
    let connections = Connections::new(pool.clone(), Credentials::new([7; 32]));
    let input = ConfigureProvider {
        request_id: Uuid::new_v4(),
        expected_revision: 0,
        provider: cubebot_protocol::runtime::ProviderKind::Openai,
        model: "test-model".into(),
        context_window: Some(200_000),
        api_key: Some("secret-provider-key".into()),
        access: None,
    };
    assert!(matches!(
        connections.configure(&member, &input).await,
        Err(IdentityError::Forbidden)
    ));
    let configured = connections.configure(&admin, &input).await.unwrap();
    assert_eq!(configured.revision, 1);
    assert_eq!(
        connections
            .configure(&admin, &input)
            .await
            .unwrap()
            .revision,
        1
    );
    assert!(matches!(
        connections.inspect(&member).await,
        Err(IdentityError::Forbidden)
    ));
    let encrypted: Vec<u8> = sqlx::query_scalar!(
        "
        SELECT credential
        FROM provider_connection
        "
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    let key = input.api_key.as_deref().unwrap();
    assert!(
        !encrypted
            .windows(key.len())
            .any(|window| window == key.as_bytes())
    );
    let restarted = Connections::new(pool, Credentials::new([7; 32]));
    assert_eq!(
        restarted.inspect(&admin).await.unwrap().unwrap().model,
        "test-model"
    );
    let stale = ConfigureProvider {
        request_id: Uuid::new_v4(),
        ..input
    };
    assert!(matches!(
        restarted.configure(&admin, &stale).await,
        Err(IdentityError::Conflict)
    ));
    // OpenAI cannot report its window; Anthropic reports its own.
    let windowless = ConfigureProvider {
        request_id: Uuid::new_v4(),
        expected_revision: 1,
        context_window: None,
        ..stale
    };
    assert!(matches!(
        restarted.configure(&admin, &windowless).await,
        Err(IdentityError::InvalidInput)
    ));
    let anthropic = ConfigureProvider {
        provider: cubebot_protocol::runtime::ProviderKind::Anthropic,
        ..windowless
    };
    restarted.configure(&admin, &anthropic).await.unwrap();
    let current = restarted.inspect(&admin).await.unwrap().unwrap();
    assert_eq!(
        current.provider,
        cubebot_protocol::runtime::ProviderKind::Anthropic
    );
    assert_eq!((current.revision, current.context_window), (2, None));
    // A model change alone keeps the saved key, re-encrypted under the next revision.
    let model_only = ConfigureProvider {
        request_id: Uuid::new_v4(),
        expected_revision: 2,
        model: "other-model".into(),
        api_key: None,
        ..anthropic
    };
    restarted.configure(&admin, &model_only).await.unwrap();
    assert!(restarted.key_matches().await.unwrap());
    assert_eq!(
        restarted.inspect(&admin).await.unwrap().unwrap().model,
        "other-model"
    );
    // A saved key never moves to another provider.
    let switched = ConfigureProvider {
        request_id: Uuid::new_v4(),
        expected_revision: 3,
        provider: cubebot_protocol::runtime::ProviderKind::Openai,
        context_window: Some(200_000),
        ..model_only
    };
    assert!(matches!(
        restarted.configure(&admin, &switched).await,
        Err(IdentityError::InvalidInput)
    ));
}
