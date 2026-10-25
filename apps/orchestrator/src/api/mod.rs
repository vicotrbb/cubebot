//! HTTP entry point for the Cubebot control plane.
//!
//! Domain operations, persistence, authorization, and reconciliation are not yet implemented.

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
