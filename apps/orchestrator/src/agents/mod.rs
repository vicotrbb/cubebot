mod commands;
mod definitions;
mod desktop;
pub mod kubernetes;
mod lifecycle;
mod revisions;
mod status;
mod wallpaper;
pub use status::AgentStatus;

pub use wallpaper::{WALLPAPER_BYTES, content_type, wallpaper};
pub mod runtime;
pub mod sandbox;
pub use definitions::{Accessory, Agent, AgentId, Avatar, Color, CreateAgent, Eyes, Paint};
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
