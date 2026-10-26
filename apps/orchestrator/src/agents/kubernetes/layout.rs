//! Fixed container/mount boundaries. Network enforcement is a separate qualification gate.
use super::{Error, client::validate_namespace};
use k8s_openapi::api::core::v1::{PersistentVolumeClaimSpec, PodSpec};
use openidconnect::reqwest::Url;
use serde_json::{Value, json};

pub struct Layout {
    runtime_image: String,
    tools_image: String,
    service_account: String,
    origin: String,
    runtime_class: Option<String>,
}
impl Layout {
    pub fn new(
        runtime_image: &str,
        tools_image: &str,
        service_account: &str,
        origin: &str,
        runtime_class: Option<String>,
    ) -> Result<Self, Error> {
        validate_image(runtime_image)?;
        validate_image(tools_image)?;
        validate_namespace(service_account)?;
        if service_account == "default" {
            return Err(Error::InvalidConfiguration);
        }
        if let Some(class) = &runtime_class {
            validate_namespace(class)?;
        }
        let url = Url::parse(origin).map_err(|_| Error::InvalidConfiguration)?;
        validate_origin(&url)?;
        Ok(Self {
            runtime_image: runtime_image.into(),
            tools_image: tools_image.into(),
            service_account: service_account.into(),
            origin: url.origin().ascii_serialization(),
            runtime_class,
        })
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

    pub fn pod(&self, claim_name: &str) -> Result<PodSpec, Error> {
        validate_namespace(claim_name)?;
        let value = json!({
            "os":{"name":"linux"},"restartPolicy":"Never", "terminationGracePeriodSeconds":30,
            "automountServiceAccountToken":false,"serviceAccountName":self.service_account,
            "shareProcessNamespace":false,"hostNetwork":false,"hostPID":false,"hostIPC":false,
            "enableServiceLinks":false,"dnsPolicy":"ClusterFirst","schedulerName":"default-scheduler",
            "runtimeClassName":self.runtime_class,
            "securityContext":{"runAsNonRoot":true,"fsGroup":20000,"fsGroupChangePolicy":"OnRootMismatch","supplementalGroupsPolicy":"Strict","seccompProfile":{"type":"RuntimeDefault"}},
            "initContainers":[self.initialize("runtime",10001),self.initialize("tools",10002)],
            "containers":[self.runtime(),self.tools()],
            "volumes":[
                {"name":"workspace","persistentVolumeClaim":{"claimName":claim_name,"readOnly":false}},
                {"name":"identity","projected":{"defaultMode":288,"sources":[{"serviceAccountToken":{"audience":"cubebot-runtime","expirationSeconds":600,"path":"token"}}]}},
                {"name":"runtime-tmp","emptyDir":{"sizeLimit":"64Mi"}},
                {"name":"tools-tmp","emptyDir":{"sizeLimit":"1Gi"}},
                {"name":"browser-shm","emptyDir":{"medium":"Memory","sizeLimit":"256Mi"}}
            ]
        });
        serde_json::from_value(value).map_err(|_| Error::InvalidConfiguration)
    }

    fn initialize(&self, directory: &str, uid: i64) -> Value {
        json!({"name":format!("initialize-{directory}"),"image":self.runtime_image,"imagePullPolicy":"IfNotPresent",
            "command":["/usr/local/bin/cubebot-agent"],"args":["--initialize-storage",directory],
            "securityContext":security(uid),"resources":resources("10m","32Mi","100m","64Mi"),
            "volumeMounts":[{"name":"workspace","mountPath":"/storage"}]})
    }
    fn runtime(&self) -> Value {
        json!({"name":"runtime","image":self.runtime_image,"imagePullPolicy":"IfNotPresent",
            "command":["/usr/local/bin/cubebot-agent"],"securityContext":security(10001),
            "resources":resources("100m","128Mi","1","512Mi"),
            "env":[{"name":"CUBEBOT_ORCHESTRATOR_ORIGIN","value":self.origin},{"name":"CUBEBOT_BOOTSTRAP_TOKEN_FILE","value":"/run/cubebot/identity/token"},{"name":"CUBEBOT_JOURNAL_DIRECTORY","value":"/var/lib/cubebot"}],
            "volumeMounts":[{"name":"workspace","mountPath":"/var/lib/cubebot","subPath":"runtime"},{"name":"identity","mountPath":"/run/cubebot/identity","readOnly":true},{"name":"runtime-tmp","mountPath":"/tmp"}]})
    }
    fn tools(&self) -> Value {
        json!({"name":"tools","image":self.tools_image,"imagePullPolicy":"IfNotPresent",
            "command":["/usr/local/bin/cubebot-tool-runner"],"securityContext":security(10002),"workingDir":"/home/agent",
            "resources":resources("100m","256Mi","2","2Gi"),"env":[{"name":"HOME","value":"/home/agent"}],
            "volumeMounts":[{"name":"workspace","mountPath":"/home/agent","subPath":"tools"},{"name":"tools-tmp","mountPath":"/tmp"},{"name":"browser-shm","mountPath":"/dev/shm"}]})
    }
}
fn security(uid: i64) -> Value {
    json!({"runAsUser":uid,"runAsGroup":uid,"runAsNonRoot":true,"privileged":false,"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}})
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
