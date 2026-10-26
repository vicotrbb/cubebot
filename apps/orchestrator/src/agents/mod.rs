//! Installation-owned definitions. Runtime and Kubernetes observations remain unimplemented.
mod definitions;
pub use definitions::{Agent, AgentId, CreateAgent};
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
