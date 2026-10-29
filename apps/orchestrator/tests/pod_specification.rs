use cubebot_orchestrator::agents::{kubernetes::Layout, sandbox::Backend};
use serde_json::json;

const IMAGE: &str =
    "registry.test/sandbox@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn layout(backend: Backend) -> Layout {
    Layout::new(
        IMAGE,
        "cubebot-runtime",
        "https://cubebot.test",
        backend,
        cubebot_protocol::network::NetworkPolicy::new(vec!["10.96.0.10".parse().unwrap()]).unwrap(),
    )
    .unwrap()
}

#[test]
fn one_sandbox_pod_has_no_initializers_or_runtime_class() {
    let spec = serde_json::to_value(layout(Backend::Process).pod("workspace").unwrap()).unwrap();
    assert_eq!(spec["restartPolicy"], "Never");
    assert_eq!(spec["automountServiceAccountToken"], false);
    assert_eq!(spec["dnsPolicy"], "None");
    assert_eq!(spec["dnsConfig"]["nameservers"], json!(["10.96.0.10"]));
    assert!(spec["initContainers"].is_null());
    assert!(spec["runtimeClassName"].is_null());
    assert_eq!(
        spec["volumes"][1]["projected"]["sources"][0]["serviceAccountToken"]["audience"],
        "cubebot-runtime"
    );
}

#[test]
fn sandbox_container_runs_the_configured_backend_without_privilege() {
    let spec = serde_json::to_value(layout(Backend::Process).pod("workspace").unwrap()).unwrap();
    let containers = spec["containers"].as_array().unwrap();
    assert_eq!(containers.len(), 1);
    let sandbox = &containers[0];
    assert_eq!(sandbox["name"], "sandbox");
    assert_eq!(
        sandbox["command"],
        json!(["/usr/local/bin/cubebot-sandbox"])
    );
    assert_eq!(sandbox["args"], json!(["run", "--backend", "process"]));
    assert_eq!(
        sandbox["securityContext"],
        json!({"runAsUser":10001,"runAsGroup":10001,"runAsNonRoot":true,"privileged":false,"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"]}})
    );
    assert!(sandbox["resources"]["limits"]["devices.kubevirt.io/kvm"].is_null());
    assert_eq!(sandbox["volumeMounts"][0]["name"], "workspace");
    assert!(sandbox["volumeMounts"][0]["subPath"].is_null());
}

#[test]
fn kvm_backend_requests_its_devices_and_only_net_admin() {
    let spec = serde_json::to_value(layout(Backend::Kvm).pod("workspace").unwrap()).unwrap();
    let sandbox = &spec["containers"][0];
    assert_eq!(sandbox["args"], json!(["run", "--backend", "kvm"]));
    let limits = &sandbox["resources"]["limits"];
    assert_eq!(limits["devices.kubevirt.io/kvm"], "1");
    assert_eq!(limits["devices.kubevirt.io/tun"], "1");
    assert_eq!(
        sandbox["securityContext"],
        json!({"runAsUser":0,"runAsGroup":0,"runAsNonRoot":false,"privileged":false,"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"],"add":["NET_ADMIN"]}})
    );
    assert!(sandbox["env"].as_array().unwrap().iter().any(|item| {
        item["name"] == "CUBEBOT_GUEST_WORKSPACE"
            && item["value"] == "/var/lib/cubebot/workspace.ext4"
    }));
}

#[test]
fn terminal_flag_and_control_ca_are_explicit() {
    let spec = serde_json::to_value(layout(Backend::Process).pod("workspace").unwrap()).unwrap();
    assert!(!spec.to_string().contains("CUBEBOT_TERMINAL_ENABLED"));
    assert!(!spec.to_string().contains("control-ca"));
    let configured = layout(Backend::Process)
        .with_terminal(true)
        .with_control_ca("control-ca")
        .unwrap();
    let spec = serde_json::to_value(configured.pod("workspace").unwrap()).unwrap();
    let env = spec["containers"][0]["env"].as_array().unwrap();
    assert!(
        env.iter()
            .any(|item| item["name"] == "CUBEBOT_TERMINAL_ENABLED" && item["value"] == "1")
    );
    assert!(
        env.iter()
            .any(|item| item["name"] == "CUBEBOT_TOOL_USER" && item["value"] == "10001:10001")
    );
    assert!(
        env.iter()
            .any(|item| item["name"] == "CUBEBOT_CONTROL_CA_FILE"
                && item["value"] == "/run/cubebot/control-ca/ca.crt")
    );
    let volume = spec["volumes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "control-ca")
        .unwrap();
    assert_eq!(
        volume["configMap"]["items"],
        json!([{"key":"ca.crt","path":"ca.crt"}])
    );
}

#[test]
fn retained_workspace_claim_is_exclusive() {
    let claim = serde_json::to_value(layout(Backend::Process).workspace()).unwrap();
    assert_eq!(claim["accessModes"], json!(["ReadWriteOncePod"]));
    assert_eq!(claim["volumeMode"], "Filesystem");
}

#[test]
fn unpinned_images_and_invalid_installation_inputs_are_rejected() {
    let network = || {
        cubebot_protocol::network::NetworkPolicy::new(vec!["10.96.0.10".parse().unwrap()]).unwrap()
    };
    for (image, account, origin) in [
        ("sandbox:latest", "runtime", "https://cubebot.test"),
        (IMAGE, "default", "https://cubebot.test"),
        (IMAGE, "runtime", "http://cubebot.test"),
        (IMAGE, "runtime", "https://user:secret@cubebot.test"),
    ] {
        assert!(Layout::new(image, account, origin, Backend::Process, network()).is_err());
    }
    assert!(
        Layout::new(
            IMAGE,
            "runtime",
            "https://cubebot.test",
            Backend::Process,
            cubebot_protocol::network::NetworkPolicy::new(vec![]).unwrap()
        )
        .is_err()
    );
    assert!(layout(Backend::Process).pod("../foreign-claim").is_err());
}
