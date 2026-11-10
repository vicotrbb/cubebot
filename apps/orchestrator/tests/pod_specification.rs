use cubebot_orchestrator::agents::{kubernetes::Layout, sandbox::Runtime};
use serde_json::json;

const IMAGE: &str =
    "registry.test/sandbox@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

fn layout() -> Layout {
    Layout::new(
        IMAGE,
        "cubebot-runtime",
        "https://cubebot.test",
        cubebot_protocol::network::NetworkPolicy::new(vec!["10.96.0.10".parse().unwrap()]).unwrap(),
    )
    .unwrap()
}

#[test]
fn one_sandbox_pod_has_no_initializers_or_runtime_class() {
    let spec = serde_json::to_value(layout().pod("workspace").unwrap()).unwrap();
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
fn sandbox_container_requests_its_kvm_devices_and_only_net_admin() {
    let spec = serde_json::to_value(layout().pod("workspace").unwrap()).unwrap();
    let containers = spec["containers"].as_array().unwrap();
    assert_eq!(containers.len(), 1);
    let sandbox = &containers[0];
    assert_eq!(sandbox["name"], "sandbox");
    assert_eq!(
        sandbox["command"],
        json!(["/usr/local/bin/cubebot-sandbox"])
    );
    assert_eq!(sandbox["args"], json!(["run"]));
    for side in ["requests", "limits"] {
        let resources = &sandbox["resources"][side];
        assert_eq!(resources["devices.kubevirt.io/kvm"], "1");
        assert_eq!(resources["devices.kubevirt.io/tun"], "1");
    }
    assert_eq!(
        sandbox["securityContext"],
        json!({"runAsUser":0,"runAsGroup":0,"runAsNonRoot":false,"privileged":false,"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":true,"capabilities":{"drop":["ALL"],"add":["NET_ADMIN"]}})
    );
    assert!(sandbox["env"].as_array().unwrap().iter().any(|item| {
        item["name"] == "CUBEBOT_GUEST_WORKSPACE"
            && item["value"] == "/var/lib/cubebot/workspace.ext4"
    }));
    assert_eq!(sandbox["volumeMounts"][0]["name"], "workspace");
    assert!(sandbox["volumeMounts"][0]["subPath"].is_null());
}

#[test]
fn a_container_runtime_runs_the_agent_image_with_baseline_privileges() {
    let container = Runtime::Container { class: None };
    let configured = layout()
        .with_terminal(true)
        .with_computer_use(true)
        .with_runtime(container)
        .unwrap();
    let spec = serde_json::to_value(configured.pod("workspace").unwrap()).unwrap();
    assert!(spec["runtimeClassName"].is_null());
    let sandbox = &spec["containers"][0];
    assert_eq!(sandbox["command"], json!(["/usr/local/bin/cubebot-agent"]));
    assert_eq!(sandbox["args"], json!(["container"]));
    assert_eq!(
        sandbox["securityContext"],
        json!({"runAsUser":0,"runAsGroup":0,"runAsNonRoot":false,"privileged":false,"allowPrivilegeEscalation":false,"readOnlyRootFilesystem":false,"capabilities":{"drop":["ALL"],"add":["SETUID","SETGID","CHOWN"]}})
    );
    let text = sandbox.to_string();
    for absent in [
        "devices.kubevirt.io",
        "NET_ADMIN",
        "CUBEBOT_GUEST_WORKSPACE",
    ] {
        assert!(!text.contains(absent), "{absent}");
    }
    assert_eq!(sandbox["volumeMounts"][0]["mountPath"], "/var/lib/cubebot");
    assert!(sandbox["env"].as_array().unwrap().iter().any(|item| {
        item["name"] == "CUBEBOT_BOOTSTRAP_TOKEN_FILE"
            && item["value"] == "/run/cubebot/identity/token"
    }));
    assert!(
        spec["volumes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|volume| volume["name"] == "shm" && volume["emptyDir"]["medium"] == "Memory")
    );
    let classed = layout()
        .with_runtime(Runtime::Container {
            class: Some("gvisor".into()),
        })
        .unwrap();
    let spec = serde_json::to_value(classed.pod("workspace").unwrap()).unwrap();
    assert_eq!(spec["runtimeClassName"], "gvisor");
    let unsafe_class = Runtime::Container {
        class: Some("../node".into()),
    };
    assert!(layout().with_runtime(unsafe_class).is_err());
}

#[test]
fn terminal_flag_and_control_ca_are_explicit() {
    let spec = serde_json::to_value(layout().pod("workspace").unwrap()).unwrap();
    assert!(!spec.to_string().contains("CUBEBOT_TERMINAL_ENABLED"));
    assert!(!spec.to_string().contains("control-ca"));
    let computer_alone = layout().with_computer_use(true);
    let spec = serde_json::to_value(computer_alone.pod("workspace").unwrap()).unwrap();
    assert!(!spec.to_string().contains("CUBEBOT_COMPUTER_ENABLED"));
    let configured = layout()
        .with_terminal(true)
        .with_computer_use(true)
        .with_control_ca("control-ca")
        .unwrap();
    let spec = serde_json::to_value(configured.pod("workspace").unwrap()).unwrap();
    let env = spec["containers"][0]["env"].as_array().unwrap();
    assert!(
        env.iter()
            .any(|item| item["name"] == "CUBEBOT_COMPUTER_ENABLED" && item["value"] == "1")
    );
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
    // Only the certificate authority, never the listener's private key in the same Secret.
    assert_eq!(volume["secret"]["secretName"], "control-ca");
    assert_eq!(
        volume["secret"]["items"],
        json!([{"key":"ca.crt","path":"ca.crt"}])
    );
}

#[test]
fn resource_overrides_replace_only_the_named_defaults() {
    let configured = layout()
        .with_runtime(Runtime::Container { class: None })
        .unwrap();
    let defaults = serde_json::to_value(configured.pod("workspace").unwrap()).unwrap()["containers"]
        [0]["resources"]
        .clone();
    assert_eq!(
        defaults,
        json!({"requests":{"cpu":"100m","memory":"256Mi","ephemeral-storage":"64Mi"},"limits":{"cpu":"2","memory":"3Gi","ephemeral-storage":"1Gi"}})
    );
    let resources = serde_json::from_value(
        json!({"requests":{"memory":"1Gi"},"limits":{"cpu":"4","memory":"8Gi"}}),
    )
    .unwrap();
    let spec = serde_json::to_value(
        configured
            .with_resources(resources)
            .unwrap()
            .pod("workspace")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        spec["containers"][0]["resources"],
        json!({"requests":{"cpu":"100m","memory":"1Gi","ephemeral-storage":"64Mi"},"limits":{"cpu":"4","memory":"8Gi","ephemeral-storage":"1Gi"}})
    );
    let kvm = serde_json::from_value(json!({"limits":{"memory":"6Gi"}})).unwrap();
    let spec = serde_json::to_value(
        layout()
            .with_resources(kvm)
            .unwrap()
            .pod("workspace")
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        spec["containers"][0]["resources"]["limits"]["memory"],
        "6Gi"
    );
    assert_eq!(
        spec["containers"][0]["resources"]["limits"]["devices.kubevirt.io/kvm"],
        "1"
    );
}

#[test]
fn resource_overrides_refuse_unknown_keys_bad_quantities_and_requests_over_limits() {
    for resources in [
        json!({"limits":{"nvidia.com/gpu":"1"}}),
        json!({"limits":{"devices.kubevirt.io/kvm":"2"}}),
        json!({"limits":{"memory":"lots"}}),
        json!({"limits":{"memory":"1.Gi"}}),
        json!({"limits":{"memory":".5Gi"}}),
        json!({"limits":{"memory":"Gi"}}),
        json!({"limits":{"memory":"-1Gi"}}),
        json!({"requests":{"memory":"4Gi"}}),
        json!({"requests":{"cpu":"3"}}),
        json!({"requests":{"cpu":"1"},"limits":{"cpu":"500m"}}),
    ] {
        let parsed = serde_json::from_value(resources.clone()).unwrap();
        assert!(layout().with_resources(parsed).is_err(), "{resources}");
    }
    let fine = serde_json::from_value(json!({"requests":{"cpu":"1.5"},"limits":{"cpu":"2"}}));
    assert!(layout().with_resources(fine.unwrap()).is_ok());
}

fn guest_memory(resources: serde_json::Value) -> Option<serde_json::Value> {
    let parsed = serde_json::from_value(resources).unwrap();
    let spec = serde_json::to_value(
        layout()
            .with_resources(parsed)
            .unwrap()
            .pod("workspace")
            .unwrap(),
    )
    .unwrap();
    spec["containers"][0]["env"]
        .as_array()
        .unwrap()
        .iter()
        .find(|env| env["name"] == "CUBEBOT_GUEST_MEMORY_MIB")
        .map(|env| env["value"].clone())
}

#[test]
fn a_kvm_guest_is_sized_by_the_memory_request_less_the_vmm() {
    // The default guest leaves the spec as it was, so upgrading replaces no agent.
    assert_eq!(guest_memory(json!({})), None);
    assert_eq!(guest_memory(json!({"requests":{"memory":"2304Mi"}})), None);
    assert_eq!(
        guest_memory(json!({"requests":{"memory":"1280Mi"},"limits":{"memory":"1536Mi"}})),
        Some(json!("1024"))
    );
    assert_eq!(
        guest_memory(json!({"requests":{"memory":"3Gi"}})),
        Some(json!("2816"))
    );
    for memory in ["511Mi", "3329Mi"] {
        let parsed =
            serde_json::from_value(json!({"requests":{"memory":memory},"limits":{"memory":"4Gi"}}))
                .unwrap();
        assert!(layout().with_resources(parsed).is_err(), "{memory}");
    }
}

#[test]
fn retained_workspace_claim_is_exclusive() {
    let claim = serde_json::to_value(layout().workspace()).unwrap();
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
        assert!(Layout::new(image, account, origin, network()).is_err());
    }
    assert!(
        Layout::new(
            IMAGE,
            "runtime",
            "https://cubebot.test",
            cubebot_protocol::network::NetworkPolicy::new(vec![]).unwrap()
        )
        .is_err()
    );
    assert!(layout().pod("../foreign-claim").is_err());
}
