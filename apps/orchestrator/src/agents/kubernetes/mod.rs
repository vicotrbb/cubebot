//! Managed namespaced resources, runtime token review and conservative termination checks.
mod authentication;
mod client;
pub use authentication::ReviewedPod;
mod observation;
mod reconcile;
mod recovery;
mod shutdown;
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
    Unauthorized,
    Busy,
    DeadlineExceeded,
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
