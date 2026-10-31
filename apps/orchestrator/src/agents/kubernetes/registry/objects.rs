use super::{Attempt, AttemptId, Error, ResourceKind, anchor::Binding};
use crate::agents::AgentId;
use k8s_openapi::{
    api::core::v1::{PersistentVolumeClaim, PersistentVolumeClaimSpec, Pod, PodSpec},
    apimachinery::pkg::apis::meta::v1::{ObjectMeta, OwnerReference},
};
use serde::Serialize;
use std::collections::BTreeMap;

pub enum ResourceSpec {
    Pod(Box<PodSpec>),
    Workspace(Box<PersistentVolumeClaimSpec>),
}
#[derive(Clone, Serialize)]
#[serde(untagged)]
pub enum ResourceObject {
    Pod(Box<Pod>),
    Workspace(Box<PersistentVolumeClaim>),
}

impl ResourceSpec {
    pub(super) fn kind(&self) -> ResourceKind {
        match self {
            Self::Pod(_) => ResourceKind::Pod,
            Self::Workspace(_) => ResourceKind::Workspace,
        }
    }
    pub(super) fn matches(&self, object: &ResourceObject) -> bool {
        match (self, object) {
            (Self::Pod(spec), ResourceObject::Pod(pod)) => pod.spec.as_ref() == Some(spec.as_ref()),
            (Self::Workspace(spec), ResourceObject::Workspace(claim)) => {
                claim.spec.as_ref() == Some(spec.as_ref())
            }
            _ => false,
        }
    }
    pub(super) fn object(
        self,
        binding: &Binding,
        agent: AgentId,
        id: AttemptId,
        epoch: i64,
    ) -> Result<ResourceObject, Error> {
        let kind = self.kind();
        let suffix = match kind {
            ResourceKind::Pod => epoch.to_string(),
            ResourceKind::Workspace => "workspace".into(),
        };
        let finalizer = match kind {
            ResourceKind::Pod => super::super::ownership::FINALIZER,
            ResourceKind::Workspace => "cubebot.dev/workspace-retained",
        };
        let metadata = ObjectMeta {
            name: Some(format!("cb-{}-{suffix}", agent.0.simple())),
            namespace: Some(binding.namespace.clone()),
            labels: Some(BTreeMap::from([
                (
                    "cubebot.dev/installation".into(),
                    binding.installation.to_string(),
                ),
                ("cubebot.dev/agent".into(), agent.0.to_string()),
                ("cubebot.dev/attempt".into(), id.0.to_string()),
            ])),
            owner_references: Some(vec![OwnerReference {
                api_version: "v1".into(),
                kind: "ConfigMap".into(),
                name: binding.name.clone(),
                uid: binding.uid.clone(),
                controller: Some(true),
                block_owner_deletion: None,
            }]),
            finalizers: Some(vec![finalizer.into()]),
            ..Default::default()
        };
        let object = match self {
            Self::Pod(spec) => ResourceObject::Pod(Box::new(Pod {
                metadata,
                spec: Some(*spec),
                ..Default::default()
            })),
            Self::Workspace(spec) => ResourceObject::Workspace(Box::new(PersistentVolumeClaim {
                metadata,
                spec: Some(*spec),
                ..Default::default()
            })),
        };
        if serde_json::to_vec(&object)
            .map_err(|error| Error::Configuration(error.into()))?
            .len()
            > 262144
        {
            return Err(Error::InvalidConfiguration);
        }
        Ok(object)
    }
}
impl ResourceObject {
    pub fn metadata(&self) -> &ObjectMeta {
        match self {
            Self::Pod(pod) => &pod.metadata,
            Self::Workspace(claim) => &claim.metadata,
        }
    }
    pub(super) fn decode(kind: ResourceKind, value: serde_json::Value) -> Result<Self, Error> {
        match kind {
            ResourceKind::Pod => serde_json::from_value(value).map(|pod| Self::Pod(Box::new(pod))),
            ResourceKind::Workspace => {
                serde_json::from_value(value).map(|claim| Self::Workspace(Box::new(claim)))
            }
        }
        .map_err(|error| Error::Configuration(error.into()))
    }
}

pub(super) fn verify(expected: &Attempt, observed: &ResourceObject) -> Result<(), Error> {
    verify_metadata(expected.object.metadata(), observed.metadata())?;
    if expected
        .uid
        .as_deref()
        .is_some_and(|uid| observed.metadata().uid.as_deref() != Some(uid))
    {
        return Err(Error::OwnershipMismatch);
    }
    let (planned, actual) = match (&expected.object, observed) {
        (ResourceObject::Pod(planned), ResourceObject::Pod(actual)) => (
            serde_json::to_value(&planned.spec),
            serde_json::to_value(&actual.spec),
        ),
        (ResourceObject::Workspace(planned), ResourceObject::Workspace(actual)) => (
            serde_json::to_value(&planned.spec),
            serde_json::to_value(&actual.spec),
        ),
        _ => return Err(Error::OwnershipMismatch),
    };
    let planned = planned.map_err(|error| Error::Configuration(error.into()))?;
    let mut actual = actual.map_err(|error| Error::Configuration(error.into()))?;
    let assigned = match expected.kind {
        ResourceKind::Pod => "nodeName",
        ResourceKind::Workspace => "volumeName",
    };
    if planned.get(assigned).is_none()
        && let Some(actual) = actual.as_object_mut()
    {
        actual.remove(assigned);
    }
    if planned != actual {
        return Err(Error::OwnershipMismatch);
    }
    Ok(())
}

pub(super) fn verify_metadata(expected: &ObjectMeta, observed: &ObjectMeta) -> Result<(), Error> {
    if expected.name != observed.name
        || expected.namespace != observed.namespace
        || expected
            .uid
            .as_deref()
            .is_some_and(|uid| observed.uid.as_deref() != Some(uid))
    {
        return Err(Error::OwnershipMismatch);
    }
    let actual = observed.labels.as_ref().ok_or(Error::OwnershipMismatch)?;
    if !expected.labels.as_ref().is_some_and(|labels| {
        labels
            .iter()
            .all(|(key, value)| actual.get(key) == Some(value))
    }) {
        return Err(Error::OwnershipMismatch);
    }
    let owners = observed.owner_references.as_deref().unwrap_or_default();
    let owned = expected
        .owner_references
        .as_deref()
        .unwrap_or_default()
        .iter()
        .all(|expected| {
            owners.iter().any(|actual| {
                actual.uid == expected.uid
                    && actual.name == expected.name
                    && actual.kind == expected.kind
                    && actual.api_version == expected.api_version
                    && actual.controller == expected.controller
            })
        });
    let finalizers = observed.finalizers.as_deref().unwrap_or_default();
    let retained = expected
        .finalizers
        .as_deref()
        .unwrap_or_default()
        .iter()
        .all(|expected| finalizers.contains(expected));
    if !owned || !retained {
        return Err(Error::OwnershipMismatch);
    }
    Ok(())
}

pub(super) fn uid(metadata: &ObjectMeta) -> Result<&str, Error> {
    let uid = metadata.uid.as_deref().ok_or(Error::OwnershipMismatch)?;
    if uid.is_empty()
        || uid.len() > 255
        || uid
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(Error::OwnershipMismatch);
    }
    Ok(uid)
}
