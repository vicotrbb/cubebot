//! One binary: each test target is a separate link and, on macOS, a first-run scan.

#[path = "support/kubernetes.rs"]
mod fixtures;
#[path = "support/registry.rs"]
mod registry_support;
#[path = "support/runtime.rs"]
mod runtime_support;
mod support;
mod tls;

mod access;
mod agent_lifecycle;
mod agents;
mod audit;
mod authentication_limits;
mod chat;
mod collaboration;
mod console;
mod controller_config;
mod credential_encryption;
mod desktop_http;
mod docker_sandbox;
mod dreams;
mod files;
mod handoffs;
mod http;
mod identity;
mod identity_http;
mod identity_requests;
mod invitations;
mod kubernetes_http;
mod kubernetes_policy;
mod mcp;
mod memories;
mod memory_agent;
mod memory_http;
mod oidc_config;
mod orchestrator_owner;
mod persistence;
mod pod_specification;
mod provider_configuration;
mod reconciliation;
mod resource_registry;
mod rooms;
mod routines;
mod runtime_identity;
mod runtime_token_review;
mod skills;
mod vault;
mod wallpaper;

/// The application endpoints without OIDC, credentials or the browser CSRF layer.
fn database_router(pool: sqlx::PgPool) -> axum::Router {
    let identity = cubebot_orchestrator::identity::Identity::new(pool.clone());
    cubebot_orchestrator::api::identity_router(pool, identity)
}
