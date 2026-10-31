//! Fixed sandbox pod and workspace shape. Network enforcement is a separate qualification gate.
use super::{Error, client::validate_namespace};
use cubebot_protocol::network::NetworkPolicy;
use k8s_openapi::{
    api::core::v1::{PersistentVolumeClaimSpec, PodSpec, VolumeResourceRequirements},
    apimachinery::pkg::api::resource::Quantity,
};
use openidconnect::reqwest::Url;
use serde_json::{Value, json};
use std::collections::BTreeMap;

pub struct Layout {
    image: String,
    service_account: String,
    origin: String,
    network: NetworkPolicy,
    control_ca: Option<String>,
    terminal: bool,
    computer_use: bool,
}
impl Layout {
    pub fn new(
        image: &str,
        service_account: &str,
        origin: &str,
        network: NetworkPolicy,
    ) -> Result<Self, Error> {
        validate_image(image)?;
        validate_namespace(service_account)?;
        if service_account == "default" || network.dns().is_empty() {
            return Err(Error::InvalidConfiguration);
        }
        let url = Url::parse(origin).map_err(|error| Error::Configuration(error.into()))?;
        validate_origin(&url)?;
        Ok(Self {
            image: image.into(),
            service_account: service_account.into(),
            origin: url.origin().ascii_serialization(),
            network,
            control_ca: None,
            terminal: false,
            computer_use: false,
        })
    }

    pub fn with_terminal(mut self, enabled: bool) -> Self {
        self.terminal = enabled;
        self
    }
    /// Computer use needs the tool user, so it takes effect only with the terminal.
    pub fn with_computer_use(mut self, enabled: bool) -> Self {
        self.computer_use = enabled;
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
            resources: Some(VolumeResourceRequirements {
                requests: Some(BTreeMap::from([(
                    "storage".into(),
                    Quantity("10Gi".into()),
                )])),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    /// One `cubebot-sandbox` container per agent (ADR 0084); the agent runs inside it.
    pub fn pod(&self, claim_name: &str) -> Result<PodSpec, Error> {
        validate_namespace(claim_name)?;
        let mut env = Vec::new();
        if self.terminal {
            env.extend([
                json!({"name":"CUBEBOT_TERMINAL_ENABLED","value":"1"}),
                json!({"name":"CUBEBOT_TOOL_USER","value":"10001:10001"}),
            ]);
            if self.computer_use {
                env.push(json!({"name":"CUBEBOT_COMPUTER_ENABLED","value":"1"}));
            }
        }
        let mut mounts = Vec::new();
        let mut volumes = vec![
            json!({"name":"workspace","persistentVolumeClaim":{"claimName":claim_name}}),
            json!({"name":"identity","projected":{"defaultMode":288,"sources":[{"serviceAccountToken":{"audience":super::authentication::RUNTIME_AUDIENCE,"expirationSeconds":600,"path":"token"}}]}}),
            json!({"name":"tmp","emptyDir":{"sizeLimit":"1Gi"}}),
        ];
        if let Some(name) = &self.control_ca {
            env.push(
                json!({"name":"CUBEBOT_CONTROL_CA_FILE","value":"/run/cubebot/control-ca/ca.crt"}),
            );
            mounts.push(
                json!({"name":"control-ca","mountPath":"/run/cubebot/control-ca","readOnly":true}),
            );
            volumes.push(json!({"name":"control-ca","configMap":{"name":name,"defaultMode":420,"items":[{"key":"ca.crt","path":"ca.crt"}]}}));
        }
        let value = json!({
            "os":{"name":"linux"},"restartPolicy":"Never", "terminationGracePeriodSeconds":30,
            "automountServiceAccountToken":false,"serviceAccountName":self.service_account,
            "shareProcessNamespace":false,
            "enableServiceLinks":false,"dnsPolicy":"None","dnsConfig":{"nameservers":self.network.dns().iter().map(ToString::to_string).collect::<Vec<_>>()},"schedulerName":"default-scheduler",
            "securityContext":{"runAsNonRoot":true,"fsGroup":20000,"fsGroupChangePolicy":"OnRootMismatch","supplementalGroupsPolicy":"Strict","seccompProfile":{"type":"RuntimeDefault"}},
            "containers":[self.sandbox(env, mounts)],
            "volumes":volumes
        });
        serde_json::from_value(value).map_err(|error| Error::Configuration(error.into()))
    }

    /// The memory limit holds the guest's default 2 GiB (ADR 0085) and the VMM around it;
    /// `env` and `mounts` follow the fixed entries.
    fn sandbox(&self, extra_env: Vec<Value>, extra_mounts: Vec<Value>) -> Value {
        let mut env = vec![
            json!({"name":"CUBEBOT_ORCHESTRATOR_ORIGIN","value":self.origin}),
            json!({"name":"CUBEBOT_BOOTSTRAP_TOKEN_FILE","value":"/run/cubebot/identity/token"}),
            json!({"name":"CUBEBOT_JOURNAL_DIRECTORY","value":"/var/lib/cubebot/runtime"}),
            json!({"name":"CUBEBOT_WORKSPACE_DIRECTORY","value":"/var/lib/cubebot/workspace"}),
            json!({"name":"CUBEBOT_GUEST_WORKSPACE","value":"/var/lib/cubebot/workspace.ext4"}),
        ];
        env.extend(extra_env);
        let mut mounts = vec![
            json!({"name":"workspace","mountPath":"/var/lib/cubebot"}),
            json!({"name":"identity","mountPath":"/run/cubebot/identity","readOnly":true}),
            json!({"name":"tmp","mountPath":"/tmp"}),
        ];
        mounts.extend(extra_mounts);
        json!({"name":"sandbox","image":self.image,"imagePullPolicy":"IfNotPresent",
            "command":["/usr/local/bin/cubebot-sandbox"],"args":["run"],
            "securityContext":security(),"resources":resources("100m","256Mi","2","3Gi"),
            "env":env,"volumeMounts":mounts})
    }
}
/// The sandbox creates the guest's tap and its tc redirects in the pod network namespace, which
/// takes root with NET_ADMIN and no other capability; the namespace's Pod Security level must be
/// privileged. fsGroup still lets it write the workspace volume.
fn security() -> Value {
    json!({"runAsUser":0,"runAsGroup":0,"runAsNonRoot":false,"privileged":false,"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"],"add":["NET_ADMIN"]}})
}
/// The API server copies extended-resource limits into requests; stating both keeps the prepared
/// preview equal to the intent.
fn resources(cpu: &str, memory: &str, max_cpu: &str, max_memory: &str) -> Value {
    json!({"requests":{"cpu":cpu,"memory":memory,"ephemeral-storage":"64Mi","devices.kubevirt.io/kvm":"1","devices.kubevirt.io/tun":"1"},"limits":{"cpu":max_cpu,"memory":max_memory,"ephemeral-storage":"1Gi","devices.kubevirt.io/kvm":"1","devices.kubevirt.io/tun":"1"}})
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
