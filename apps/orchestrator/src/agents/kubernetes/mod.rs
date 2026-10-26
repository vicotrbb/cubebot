//! Namespaced Kubernetes effects and conservative ownership/termination checks.
mod client;
mod ownership;
pub use client::Kubernetes;
pub use ownership::{Identity, RecordedPod, termination_proven, verify_pod};

#[derive(Debug)]
pub enum Error {
    InvalidConfiguration,
    OwnershipMismatch,
    TerminationUnproven,
    Api(kube::Error),
}

impl From<kube::Error> for Error {
    fn from(error: kube::Error) -> Self {
        Self::Api(error)
    }
}
