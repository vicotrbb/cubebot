//! Installation-owned definitions and explicit Kubernetes reconciliation.
mod commands;
mod definitions;
mod desktop;
pub mod kubernetes;
mod lifecycle;
mod revisions;
mod status;
pub use status::AgentStatus;
pub mod runtime;
pub mod sandbox;
pub use definitions::{Agent, AgentId, CreateAgent};
pub use lifecycle::{LifecycleAction, LifecycleCommand};
pub use revisions::{DefinitionRevision, IdlePolicy, UpdateAgent};
use sqlx::PgPool;

#[derive(Clone)]
pub struct Agents {
    pool: PgPool,
}

impl Agents {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}
