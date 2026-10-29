//! Fixed sandbox pod and workspace shape. Network enforcement is a separate qualification gate.
use super::{Error, client::validate_namespace};
use crate::agents::sandbox::Backend;
use cubebot_protocol::network::NetworkPolicy;
use k8s_openapi::api::core::v1::{PersistentVolumeClaimSpec, PodSpec};
use openidconnect::reqwest::Url;
use serde_json::{Value, json};

pub struct Layout {
    image: String,
    service_account: String,
    origin: String,
    backend: Backend,
    network: NetworkPolicy,
    control_ca: Option<String>,
    terminal: bool,
}
impl Layout {
    pub fn new(
        image: &str,
        service_account: &str,
        origin: &str,
        backend: Backend,
        network: NetworkPolicy,
    ) -> Result<Self, Error> {
        validate_image(image)?;
        validate_namespace(service_account)?;
        if service_account == "default" || network.dns().is_empty() {
            return Err(Error::InvalidConfiguration);
        }
        let url = Url::parse(origin).map_err(|_| Error::InvalidConfiguration)?;
        validate_origin(&url)?;
        Ok(Self {
            image: image.into(),
            service_account: service_account.into(),
            origin: url.origin().ascii_serialization(),
            backend,
            network,
            control_ca: None,
            terminal: false,
        })
    }

    pub fn with_terminal(mut self, enabled: bool) -> Self {
        self.terminal = enabled;
        self
    }
    pub fn with_control_ca(mut self, config_map: &str) -> Result<Self, Error> {
        if config_map.len() > 253
            || !config_map.split('.').all(|part| {
                !part.is_empty()
                    && !part.starts_with('-')
                    && !part.ends_with('-')
                    && part.bytes().all(|byte| {
                        byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                    })
            })
        {
            return Err(Error::InvalidConfiguration);
        }
        self.control_ca = Some(config_map.into());
        Ok(self)
    }

    pub fn workspace(&self) -> PersistentVolumeClaimSpec {
        PersistentVolumeClaimSpec {
            access_modes: Some(vec!["ReadWriteOncePod".into()]),
            volume_mode: Some("Filesystem".into()),
            resources: Some(
                serde_json::from_value(json!({"requests":{"storage":"10Gi"}}))
                    .expect("fixed volume resources"),
            ),
            ..Default::default()
        }
    }

    /// One `cubebot-sandbox` container per agent (ADR 0084); the agent runs inside it.
    pub fn pod(&self, claim_name: &str) -> Result<PodSpec, Error> {
        validate_namespace(claim_name)?;
        let mut value = json!({
            "os":{"name":"linux"},"restartPolicy":"Never", "terminationGracePeriodSeconds":30,
            "automountServiceAccountToken":false,"serviceAccountName":self.service_account,
            "shareProcessNamespace":false,
            "enableServiceLinks":false,"dnsPolicy":"None","dnsConfig":{"nameservers":self.network.dns().iter().map(ToString::to_string).collect::<Vec<_>>()},"schedulerName":"default-scheduler",
            "securityContext":{"runAsNonRoot":true,"fsGroup":20000,"fsGroupChangePolicy":"OnRootMismatch","supplementalGroupsPolicy":"Strict","seccompProfile":{"type":"RuntimeDefault"}},
            "containers":[self.sandbox()],
            "volumes":[
                {"name":"workspace","persistentVolumeClaim":{"claimName":claim_name}},
                {"name":"identity","projected":{"defaultMode":288,"sources":[{"serviceAccountToken":{"audience":super::authentication::RUNTIME_AUDIENCE,"expirationSeconds":600,"path":"token"}}]}},
                {"name":"tmp","emptyDir":{"sizeLimit":"1Gi"}}
            ]
        });
        let sandbox = &mut value["containers"][0];
        if self.terminal {
            // shortcut: the process backend cannot switch users, so tools run as the sandbox's
            // own user; the kvm guest will name a separate one.
            sandbox["env"]
                .as_array_mut()
                .expect("fixed environment")
                .extend([
                    json!({"name":"CUBEBOT_TERMINAL_ENABLED","value":"1"}),
                    json!({"name":"CUBEBOT_TOOL_USER","value":"10001:10001"}),
                ]);
        }
        if self.backend == Backend::Kvm {
            // The sandbox creates the guest's tap and its tc redirects in the pod network namespace,
            // which takes root with NET_ADMIN and no other capability; the namespace's Pod Security
            // level must be privileged. fsGroup still lets it write the workspace volume.
            let limits = &mut sandbox["resources"]["limits"];
            limits["devices.kubevirt.io/kvm"] = json!("1");
            limits["devices.kubevirt.io/tun"] = json!("1");
            sandbox["securityContext"] = security(0);
            sandbox["securityContext"]["capabilities"]["add"] = json!(["NET_ADMIN"]);
            sandbox["env"].as_array_mut().expect("fixed environment").push(json!({"name":"CUBEBOT_GUEST_WORKSPACE","value":"/var/lib/cubebot/workspace.ext4"}));
        }
        if let Some(name) = &self.control_ca {
            sandbox["env"].as_array_mut().expect("fixed environment").push(json!({"name":"CUBEBOT_CONTROL_CA_FILE","value":"/run/cubebot/control-ca/ca.crt"}));
            sandbox["volumeMounts"].as_array_mut().expect("fixed mounts").push(json!({"name":"control-ca","mountPath":"/run/cubebot/control-ca","readOnly":true}));
            value["volumes"].as_array_mut().expect("fixed volumes").push(json!({"name":"control-ca","configMap":{"name":name,"defaultMode":420,"items":[{"key":"ca.crt","path":"ca.crt"}]}}));
        }
        serde_json::from_value(value).map_err(|_| Error::InvalidConfiguration)
    }

    fn sandbox(&self) -> Value {
        json!({"name":"sandbox","image":self.image,"imagePullPolicy":"IfNotPresent",
            "command":["/usr/local/bin/cubebot-sandbox"],"args":["run","--backend",self.backend.as_str()],
            "securityContext":security(10001),"resources":resources("100m","256Mi","2","2Gi"),
            "env":[{"name":"CUBEBOT_ORCHESTRATOR_ORIGIN","value":self.origin},{"name":"CUBEBOT_BOOTSTRAP_TOKEN_FILE","value":"/run/cubebot/identity/token"},{"name":"CUBEBOT_JOURNAL_DIRECTORY","value":"/var/lib/cubebot/runtime"},{"name":"CUBEBOT_WORKSPACE_DIRECTORY","value":"/var/lib/cubebot/workspace"}],
            "volumeMounts":[{"name":"workspace","mountPath":"/var/lib/cubebot"},{"name":"identity","mountPath":"/run/cubebot/identity","readOnly":true},{"name":"tmp","mountPath":"/tmp"}]})
    }
}
fn security(uid: i64) -> Value {
    json!({"runAsUser":uid,"runAsGroup":uid,"runAsNonRoot":uid != 0,"privileged":false,"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}})
}
fn resources(cpu: &str, memory: &str, max_cpu: &str, max_memory: &str) -> Value {
    json!({"requests":{"cpu":cpu,"memory":memory,"ephemeral-storage":"64Mi"},"limits":{"cpu":max_cpu,"memory":max_memory,"ephemeral-storage":"1Gi"}})
}
fn validate_image(image: &str) -> Result<(), Error> {
    let Some((repository, digest)) = image.split_once("@sha256:") else {
        return Err(Error::InvalidConfiguration);
    };
    if repository.is_empty()
        || image.len() > 512
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
        || image
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(Error::InvalidConfiguration);
    }
    Ok(())
}
fn validate_origin(url: &Url) -> Result<(), Error> {
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.path() != "/"
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(Error::InvalidConfiguration);
    }
    Ok(())
}
