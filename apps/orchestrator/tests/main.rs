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
mod authentication_limits;
mod chat;
mod controller_config;
mod credential_encryption;
mod docker_sandbox;
mod http;
mod identity;
mod identity_http;
mod identity_requests;
mod invitations;
mod kubernetes_http;
mod kubernetes_policy;
mod oidc_config;
mod orchestrator_owner;
mod persistence;
mod pod_specification;
mod provider_configuration;
mod reconciliation;
mod resource_registry;
mod runtime_identity;
mod runtime_token_review;
