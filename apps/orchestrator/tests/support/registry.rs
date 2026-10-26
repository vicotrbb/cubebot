use cubebot_orchestrator::{
    agents::{
        AgentId, Agents, CreateAgent, LifecycleAction, LifecycleCommand, kubernetes::Registry,
    },
    identity::Identity,
};
use uuid::Uuid;
pub async fn setup() -> (sqlx::PgPool, Registry, AgentId) {
    let pool = crate::support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let actor = identity.authenticate(&session.token).await.unwrap();
    let agents = Agents::new(pool.clone());
    let created = agents
        .create(
            &actor,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: "Registry agent".into(),
                instructions: "Preserve identity".into(),
            },
        )
        .await
        .unwrap();
    let agent = AgentId(created.id);
    agents
        .lifecycle(
            &actor,
            agent,
            &LifecycleCommand {
                request_id: Uuid::new_v4(),
                expected_revision: 1,
                action: LifecycleAction::Wake,
            },
        )
        .await
        .unwrap();
    let registry = Registry::new(pool.clone());
    (pool, registry, agent)
}
