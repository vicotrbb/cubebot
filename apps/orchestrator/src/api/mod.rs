//! HTTP entry point for the Cubebot control plane.
//!
//! Database-enabled mode exposes the initial authenticated identity endpoints.

mod identity;
mod invitations;

use axum::{Json, Router, routing::get};
use cubebot_protocol::{Liveness, LivenessStatus};

pub fn router() -> Router {
    Router::new().route("/api/health/live", get(liveness))
}

async fn liveness() -> Json<Liveness> {
    Json(Liveness {
        status: LivenessStatus::Alive,
    })
}

/// Configured application endpoints share one concrete PostgreSQL pool.
pub fn database_router(pool: sqlx::PgPool) -> Router {
    router()
        .merge(identity::router(crate::identity::Identity::new(
            pool.clone(),
        )))
        .merge(invitations::router(crate::identity::Identity::new(
            pool.clone(),
        )))
        .route("/api/health/ready", get(readiness).with_state(pool))
}

async fn readiness(
    axum::extract::State(pool): axum::extract::State<sqlx::PgPool>,
) -> axum::http::StatusCode {
    match sqlx::query("SELECT 1").execute(&pool).await {
        Ok(_) => axum::http::StatusCode::OK,
        Err(_) => axum::http::StatusCode::SERVICE_UNAVAILABLE,
    }
}
