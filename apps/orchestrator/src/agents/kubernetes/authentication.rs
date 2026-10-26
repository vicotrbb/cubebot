//! Kubernetes-attested pod identity, separate from Cubebot execution authority.
use super::{Error, client::validate_namespace};
use k8s_openapi::api::authentication::v1::TokenReviewStatus;
use serde::Serialize;

pub(super) const RUNTIME_AUDIENCE: &str = "cubebot-runtime";

#[derive(Serialize)]
pub struct ReviewedPod {
    pub(in crate::agents) namespace: String,
    pub(in crate::agents) service_account: String,
    pub(in crate::agents) service_account_uid: String,
    pub(in crate::agents) pod_name: String,
    pub(in crate::agents) pod_uid: String,
}
pub(super) fn verify(status: TokenReviewStatus, namespace: &str) -> Result<ReviewedPod, Error> {
    if status.authenticated != Some(true)
        || status.error.is_some()
        || !status
            .audiences
            .as_ref()
            .is_some_and(|values| values.iter().any(|value| value == RUNTIME_AUDIENCE))
    {
        return Err(Error::Unauthorized);
    }
    let user = status.user.ok_or(Error::Unauthorized)?;
    let prefix = format!("system:serviceaccount:{namespace}:");
    let service_account = user
        .username
        .as_deref()
        .and_then(|value| value.strip_prefix(&prefix))
        .ok_or(Error::Unauthorized)?;
    validate_namespace(service_account).map_err(|_| Error::Unauthorized)?;
    let service_account_uid = user
        .uid
        .filter(|value| valid_uid(value))
        .ok_or(Error::Unauthorized)?;
    let extra = user.extra.ok_or(Error::Unauthorized)?;
    let pod_name = single(extra.get("authentication.kubernetes.io/pod-name"))?;
    validate_namespace(&pod_name).map_err(|_| Error::Unauthorized)?;
    let pod_uid = single(extra.get("authentication.kubernetes.io/pod-uid"))?;
    if !valid_uid(&pod_uid) {
        return Err(Error::Unauthorized);
    }
    Ok(ReviewedPod {
        namespace: namespace.into(),
        service_account: service_account.into(),
        service_account_uid,
        pod_name,
        pod_uid,
    })
}
fn single(values: Option<&Vec<String>>) -> Result<String, Error> {
    let values = values.ok_or(Error::Unauthorized)?;
    if values.len() != 1 {
        return Err(Error::Unauthorized);
    }
    Ok(values[0].clone())
}
fn valid_uid(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 255
        && !value
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
}
