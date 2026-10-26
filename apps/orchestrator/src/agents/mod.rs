//! Installation-owned definitions. Runtime and Kubernetes observations remain unimplemented.
mod commands;
mod definitions;
mod lifecycle;
mod revisions;
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
