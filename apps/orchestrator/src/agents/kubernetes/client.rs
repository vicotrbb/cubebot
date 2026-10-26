use super::{Error, RecordedPod, ownership::FINALIZER, termination_proven, verify_pod};
use k8s_openapi::api::core::v1::{PersistentVolumeClaim, Pod};
use kube::{
    Api, Client,
    api::{DeleteParams, Patch, PatchParams, PostParams, Preconditions},
};

#[derive(Clone)]
pub struct Kubernetes {
    pods: Api<Pod>,
    workspaces: Api<PersistentVolumeClaim>,
}

impl Kubernetes {
    pub fn new(client: Client, namespace: &str) -> Result<Self, Error> {
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
        Ok(Self {
            pods: Api::namespaced(client.clone(), namespace),
            workspaces: Api::namespaced(client, namespace),
        })
    }

    pub async fn create_pod(&self, pod: &Pod) -> Result<Pod, Error> {
        Ok(self.pods.create(&PostParams::default(), pod).await?)
    }

    pub async fn create_workspace(
        &self,
        claim: &PersistentVolumeClaim,
    ) -> Result<PersistentVolumeClaim, Error> {
        Ok(self
            .workspaces
            .create(&PostParams::default(), claim)
            .await?)
    }

    pub async fn observe(&self, recorded: &RecordedPod) -> Result<Option<Pod>, Error> {
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
        if !pod
            .metadata
            .finalizers
            .as_ref()
            .is_some_and(|values| values.iter().any(|value| value == FINALIZER))
        {
            return Err(Error::OwnershipMismatch);
        }
        let revision = pod
            .metadata
            .resource_version
            .ok_or(Error::OwnershipMismatch)?;
        self.pods
            .delete(&recorded.name, &delete_params(recorded, revision))
            .await?;
        Ok(())
    }

    /// Never infer termination from 404 or remove another controller's finalizer.
    pub async fn finalize_stopped(&self, recorded: &RecordedPod) -> Result<(), Error> {
        let pod = self
            .observe(recorded)
            .await?
            .ok_or(Error::TerminationUnproven)?;
        if pod.metadata.deletion_timestamp.is_none() || !termination_proven(&pod) {
            return Err(Error::TerminationUnproven);
        }
        let revision = pod
            .metadata
            .resource_version
            .ok_or(Error::OwnershipMismatch)?;
        let finalizers: Vec<_> = pod
            .metadata
            .finalizers
            .unwrap_or_default()
            .into_iter()
            .filter(|value| value != FINALIZER)
            .collect();
        let patch = serde_json::json!({"metadata":{"resourceVersion":revision,"uid":recorded.uid,"finalizers":finalizers}});
        self.pods
            .patch(
                &recorded.name,
                &PatchParams::default(),
                &Patch::Merge(patch),
            )
            .await?;
        Ok(())
    }
}

fn delete_params(recorded: &RecordedPod, revision: String) -> DeleteParams {
    DeleteParams {
        grace_period_seconds: Some(30),
        preconditions: Some(Preconditions {
            uid: Some(recorded.uid.clone()),
            resource_version: Some(revision),
        }),
        ..Default::default()
    }
}
