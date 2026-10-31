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
mod collaboration;
mod controller_config;
mod credential_encryption;
mod desktop_http;
mod docker_sandbox;
mod dreams;
mod http;
mod identity;
mod identity_http;
mod identity_requests;
mod invitations;
mod kubernetes_http;
mod kubernetes_policy;
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
mod runtime_identity;
mod runtime_token_review;
mod skills;
mod wallpaper;
