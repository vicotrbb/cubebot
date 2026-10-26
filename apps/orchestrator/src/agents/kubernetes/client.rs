use super::{
    Error, RecordedPod, ResourceObject, ownership::FINALIZER, termination_proven, verify_pod,
};
use k8s_openapi::api::authentication::v1::{TokenReview, TokenReviewSpec};
use k8s_openapi::api::core::v1::{ConfigMap, PersistentVolumeClaim, Pod};
use kube::{
    Api, Client,
    api::{DeleteParams, Patch, PatchParams, PostParams, Preconditions},
};

#[derive(Clone)]
pub struct Kubernetes {
    namespace: String,
    token_reviews: Api<TokenReview>,
    review_slots: std::sync::Arc<tokio::sync::Semaphore>,
    pods: Api<Pod>,
    anchors: Api<ConfigMap>,
    workspaces: Api<PersistentVolumeClaim>,
}

impl Kubernetes {
    pub fn new(mut config: kube::Config, namespace: &str) -> Result<Self, Error> {
        validate_namespace(namespace)?;
        config.default_retry = false;
        config.connect_timeout = Some(std::time::Duration::from_secs(5));
        config.read_timeout = Some(std::time::Duration::from_secs(10));
        config.write_timeout = Some(std::time::Duration::from_secs(10));
        let client = Client::try_from(config)?;
        Ok(Self {
            namespace: namespace.into(),
            token_reviews: Api::all(client.clone()),
            review_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(16)),
            pods: Api::namespaced(client.clone(), namespace),
            anchors: Api::namespaced(client.clone(), namespace),
            workspaces: Api::namespaced(client, namespace),
        })
    }

    pub async fn review_runtime_token(&self, token: &str) -> Result<super::ReviewedPod, Error> {
        if token.is_empty()
            || token.len() > 16384
            || token
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        {
            return Err(Error::Unauthorized);
        }
        let _slot = self
            .review_slots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Busy)?;
        let review = TokenReview {
            spec: TokenReviewSpec {
                audiences: Some(vec![super::authentication::RUNTIME_AUDIENCE.into()]),
                token: token.into(),
            },
            ..Default::default()
        };
        let reviewed = self
            .token_reviews
            .create(&PostParams::default(), &review)
            .await?;
        super::authentication::verify(reviewed.status.ok_or(Error::Unauthorized)?, &self.namespace)
    }

    /// Server defaulting/admission preview, never evidence of a physical object.
    pub async fn prepare(&self, object: &ResourceObject) -> Result<ResourceObject, Error> {
        self.validate_target(object.metadata())?;
        if object.metadata().uid.is_some() || object.metadata().resource_version.is_some() {
            return Err(Error::InvalidConfiguration);
        }
        let parameters = PostParams {
            dry_run: true,
            ..Default::default()
        };
        let mut preview = match object {
            ResourceObject::Pod(pod) => {
                ResourceObject::Pod(Box::new(self.pods.create(&parameters, pod).await?))
            }
            ResourceObject::Workspace(claim) => ResourceObject::Workspace(Box::new(
                self.workspaces.create(&parameters, claim).await?,
            )),
        };
        clear_preview_observation(&mut preview);
        Ok(preview)
    }

    pub async fn create_anchor(&self, anchor: &ConfigMap) -> Result<ConfigMap, Error> {
        self.validate_target(&anchor.metadata)?;
        Ok(self.anchors.create(&PostParams::default(), anchor).await?)
    }

    /// A candidate read never grants ownership. The durable registry must verify it.
    pub async fn fetch_candidate(
        &self,
        expected: &ResourceObject,
    ) -> Result<Option<ResourceObject>, Error> {
        self.validate_target(expected.metadata())?;
        let name = expected
            .metadata()
            .name
            .as_deref()
            .ok_or(Error::InvalidConfiguration)?;
        match expected {
            ResourceObject::Pod(_) => Ok(self
                .pods
                .get_opt(name)
                .await?
                .map(|pod| ResourceObject::Pod(Box::new(pod)))),
            ResourceObject::Workspace(_) => Ok(self
                .workspaces
                .get_opt(name)
                .await?
                .map(|claim| ResourceObject::Workspace(Box::new(claim)))),
        }
    }

    /// Untrusted platform observation for installation process fencing, never adoption.
    pub(crate) async fn platform_pod(&self, name: &str) -> Result<Option<Pod>, Error> {
        validate_namespace(name)?;
        Ok(self.pods.get_opt(name).await?)
    }

    pub async fn fetch_anchor(&self, name: &str) -> Result<Option<ConfigMap>, Error> {
        validate_namespace(name)?;
        Ok(self.anchors.get_opt(name).await?)
    }

    fn validate_target(
        &self,
        metadata: &k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta,
    ) -> Result<(), Error> {
        let name = metadata
            .name
            .as_deref()
            .ok_or(Error::InvalidConfiguration)?;
        validate_namespace(name)?;
        if metadata
            .namespace
            .as_ref()
            .is_some_and(|namespace| namespace != &self.namespace)
        {
            return Err(Error::OwnershipMismatch);
        }
        Ok(())
    }

    pub async fn create_pod(&self, pod: &Pod) -> Result<Pod, Error> {
        self.validate_target(&pod.metadata)?;
        Ok(self.pods.create(&PostParams::default(), pod).await?)
    }

    pub async fn create_workspace(
        &self,
        claim: &PersistentVolumeClaim,
    ) -> Result<PersistentVolumeClaim, Error> {
        self.validate_target(&claim.metadata)?;
        Ok(self
            .workspaces
            .create(&PostParams::default(), claim)
            .await?)
    }

    pub async fn observe(&self, recorded: &RecordedPod) -> Result<Option<Pod>, Error> {
        validate_namespace(&recorded.name)?;
        if recorded.namespace != self.namespace {
            return Err(Error::OwnershipMismatch);
        }
        let pod = self.pods.get_opt(&recorded.name).await?;
        if let Some(pod) = &pod {
            verify_pod(pod, recorded)?;
        }
        Ok(pod)
    }

    /// The caller must settle/drain assigned work before requesting ordinary termination.
    pub async fn request_stop(&self, recorded: &RecordedPod) -> Result<(), Error> {
        let pod = self
            .observe(recorded)
            .await?
            .ok_or(Error::TerminationUnproven)?;
        self.stop_observed(&pod).await
    }

    pub(super) async fn stop_observed(&self, pod: &Pod) -> Result<(), Error> {
        self.validate_target(&pod.metadata)?;
        if !pod
            .metadata
            .finalizers
            .as_ref()
            .is_some_and(|values| values.iter().any(|value| value == FINALIZER))
        {
            return Err(Error::OwnershipMismatch);
        }
        let (name, uid, revision) = observed_identity(pod)?;
        self.pods
            .delete(name, &delete_params(uid, revision.to_owned()))
            .await?;
        Ok(())
    }

    /// Never infer termination from 404 or remove another controller's finalizer.
    pub async fn finalize_stopped(&self, recorded: &RecordedPod) -> Result<(), Error> {
        let pod = self
            .observe(recorded)
            .await?
            .ok_or(Error::TerminationUnproven)?;
        self.finalize_observed(&pod).await
    }

    pub(super) async fn finalize_observed(&self, pod: &Pod) -> Result<(), Error> {
        self.validate_target(&pod.metadata)?;
        if pod.metadata.deletion_timestamp.is_none() || !termination_proven(pod) {
            return Err(Error::TerminationUnproven);
        }
        let (name, uid, revision) = observed_identity(pod)?;
        let finalizers: Vec<_> = pod
            .metadata
            .finalizers
            .clone()
            .unwrap_or_default()
            .into_iter()
            .filter(|value| value != FINALIZER)
            .collect();
        let patch = serde_json::json!({"metadata":{"resourceVersion":revision,"uid":uid,"finalizers":finalizers}});
        self.pods
            .patch(name, &PatchParams::default(), &Patch::Merge(patch))
            .await?;
        Ok(())
    }
}

fn delete_params(uid: &str, revision: String) -> DeleteParams {
    DeleteParams {
        grace_period_seconds: Some(30),
        preconditions: Some(Preconditions {
            uid: Some(uid.to_owned()),
            resource_version: Some(revision),
        }),
        ..Default::default()
    }
}

pub(super) fn validate_namespace(namespace: &str) -> Result<(), Error> {
    if namespace.is_empty()
        || namespace.len() > 63
        || !namespace
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        || namespace.starts_with('-')
        || namespace.ends_with('-')
    {
        return Err(Error::InvalidConfiguration);
    }
    Ok(())
}

fn clear_preview_observation(object: &mut ResourceObject) {
    let metadata = match object {
        ResourceObject::Pod(pod) => {
            pod.status = None;
            &mut pod.metadata
        }
        ResourceObject::Workspace(claim) => {
            claim.status = None;
            &mut claim.metadata
        }
    };
    metadata.uid = None;
    metadata.generation = None;
    metadata.resource_version = None;
    metadata.creation_timestamp = None;
    metadata.deletion_timestamp = None;
    metadata.deletion_grace_period_seconds = None;
    metadata.managed_fields = None;
}

fn observed_identity(pod: &Pod) -> Result<(&str, &str, &str), Error> {
    let metadata = &pod.metadata;
    let name = metadata.name.as_deref().ok_or(Error::OwnershipMismatch)?;
    let uid = metadata
        .uid
        .as_deref()
        .filter(|uid| !uid.is_empty())
        .ok_or(Error::OwnershipMismatch)?;
    let version = metadata
        .resource_version
        .as_deref()
        .filter(|version| !version.is_empty())
        .ok_or(Error::OwnershipMismatch)?;
    Ok((name, uid, version))
}
