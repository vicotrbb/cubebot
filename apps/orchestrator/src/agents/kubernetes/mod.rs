//! Namespaced Kubernetes effects and conservative ownership/termination checks.
mod client;
mod reconcile;
pub use reconcile::{ReconcileResult, Reconciler};
mod layout;
pub use layout::Layout;
mod registry;
pub use registry::{
    Attempt, AttemptId, Phase, Registry, ResourceKind, ResourceObject, ResourceSpec,
};
mod ownership;
pub use client::Kubernetes;
pub use ownership::{Identity, RecordedPod, termination_proven, verify_pod};

#[derive(Debug)]
pub enum Error {
    InvalidConfiguration,
    Conflict,
    Database(sqlx::Error),
    OwnershipMismatch,
    TerminationUnproven,
    Api(kube::Error),
}

impl From<kube::Error> for Error {
    fn from(error: kube::Error) -> Self {
        Self::Api(error)
    }
}

impl From<sqlx::Error> for Error {
    fn from(error: sqlx::Error) -> Self {
        Self::Database(error)
    }
}
