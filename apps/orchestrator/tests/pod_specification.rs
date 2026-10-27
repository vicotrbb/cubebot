use cubebot_orchestrator::agents::kubernetes::Layout;

#[test]
fn control_ca_is_explicit_and_mounted_only_in_runtime() {
    let layout = Layout::new("registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "registry.test/tools@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "cubebot-runtime", "https://cubebot.test", None, cubebot_protocol::network::NetworkPolicy::new(vec![], vec!["10.96.0.10".parse().unwrap()]).unwrap()).unwrap();
    let layout = layout.with_control_ca("control-ca").unwrap();
    let pod = serde_json::to_value(layout.pod("workspace").unwrap()).unwrap();
    let containers = pod["containers"].as_array().unwrap();
    let runtime = containers
        .iter()
        .find(|item| item["name"] == "runtime")
        .unwrap();
    assert!(
        runtime["env"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "CUBEBOT_CONTROL_CA_FILE"
                && item["value"] == "/run/cubebot/control-ca/ca.crt")
    );
    assert!(
        runtime["volumeMounts"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["name"] == "control-ca" && item["readOnly"] == true)
    );
    let tools = containers
        .iter()
        .find(|item| item["name"] == "tools")
        .unwrap();
    assert!(!tools.to_string().contains("control-ca"));
    assert!(!pod["initContainers"].to_string().contains("control-ca"));
    let volume = pod["volumes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "control-ca")
        .unwrap();
    assert_eq!(volume["configMap"]["name"], "control-ca");
    assert_eq!(
        volume["configMap"]["items"],
        serde_json::json!([{"key":"ca.crt","path":"ca.crt"}])
    );
}
use serde_json::json;

#[test]
fn network_setup_precedes_tools_without_credential_or_workspace_access() {
    let image = "registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let layout = Layout::new(
        image,
        image,
        "cubebot-runtime",
        "https://cubebot.test",
        None,
        cubebot_protocol::network::NetworkPolicy::new(
            vec!["8.8.4.4/32".parse().unwrap()],
            vec!["10.96.0.10".parse().unwrap()],
        )
        .unwrap(),
    )
    .unwrap();
    let spec = serde_json::to_value(layout.pod("workspace").unwrap()).unwrap();
    assert_eq!(spec["dnsPolicy"], "None");
    assert_eq!(spec["dnsConfig"]["nameservers"], json!(["10.96.0.10"]));
    let init = &spec["initContainers"][0];
    assert_eq!(init["name"], "initialize-network");
    assert_eq!(init["command"], json!(["/usr/local/bin/cubebot-agent"]));
    assert_eq!(init["args"][0], "--initialize-network");
    let network: serde_json::Value =
        serde_json::from_str(init["args"][1].as_str().unwrap()).unwrap();
    assert_eq!(
        network,
        json!({"infrastructure":["8.8.4.4/32"],"dns":["10.96.0.10"]})
    );
    assert_eq!(
        init["securityContext"]["capabilities"],
        json!({"drop":["ALL"],"add":["NET_ADMIN"]})
    );
    assert_eq!(init["securityContext"]["runAsUser"], 0);
    assert_eq!(init["securityContext"]["allowPrivilegeEscalation"], false);
    assert_eq!(init["securityContext"]["readOnlyRootFilesystem"], true);
    assert!(init["volumeMounts"].is_null());
    assert!(init["env"].is_null());
    verify_supervisor_capabilities(&spec);
}

#[test]
fn compiled_layout_separates_identity_journal_and_tool_mounts() {
    let layout = Layout::new("registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "registry.test/tools@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "cubebot-runtime", "https://cubebot.test", None,cubebot_protocol::network::NetworkPolicy::new(vec![], vec!["10.96.0.10".parse().unwrap()]).unwrap()).unwrap();
    let spec = serde_json::to_value(layout.pod("workspace-claim").unwrap()).unwrap();
    assert_eq!(spec["restartPolicy"], "Never");
    assert_eq!(spec["automountServiceAccountToken"], false);
    assert_eq!(spec["shareProcessNamespace"], false);
    let runtime = &spec["containers"][0];
    let tools = &spec["containers"][1];
    assert_eq!(runtime["name"], "runtime");
    assert_eq!(tools["name"], "tools");
    assert_ne!(
        runtime["securityContext"]["runAsUser"],
        tools["securityContext"]["runAsUser"]
    );
    assert_eq!(runtime["volumeMounts"][0]["subPath"], "runtime");
    assert_eq!(tools["volumeMounts"][0]["subPath"], "tools");
    assert!(
        tools["volumeMounts"]
            .as_array()
            .unwrap()
            .iter()
            .all(|mount| mount["name"] != "identity")
    );
    assert_eq!(
        spec["volumes"][1]["projected"]["sources"][0]["serviceAccountToken"]["audience"],
        "cubebot-runtime"
    );
    verify_container_limits(runtime, tools);
    assert_eq!(
        spec["initContainers"][1]["args"],
        json!(["--initialize-storage", "runtime"])
    );
    assert_eq!(
        spec["initContainers"][2]["args"],
        json!(["--initialize-storage", "tools"])
    );
    let claim = serde_json::to_value(layout.workspace()).unwrap();
    assert_eq!(claim["accessModes"], json!(["ReadWriteOncePod"]));
    assert_eq!(claim["volumeMode"], "Filesystem");
}

#[test]
fn unpinned_images_and_invalid_installation_inputs_are_rejected() {
    let image = "registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    assert!(
        Layout::new(
            "agent:latest",
            image,
            "runtime",
            "https://cubebot.test",
            None,
            cubebot_protocol::network::NetworkPolicy::new(
                vec![],
                vec!["10.96.0.10".parse().unwrap()]
            )
            .unwrap()
        )
        .is_err()
    );
    assert!(
        Layout::new(
            image,
            image,
            "default",
            "https://cubebot.test",
            None,
            cubebot_protocol::network::NetworkPolicy::new(
                vec![],
                vec!["10.96.0.10".parse().unwrap()]
            )
            .unwrap()
        )
        .is_err()
    );
    assert!(
        Layout::new(
            image,
            image,
            "runtime",
            "http://cubebot.test",
            None,
            cubebot_protocol::network::NetworkPolicy::new(
                vec![],
                vec!["10.96.0.10".parse().unwrap()]
            )
            .unwrap()
        )
        .is_err()
    );
    assert!(
        Layout::new(
            image,
            image,
            "runtime",
            "https://user:secret@cubebot.test",
            None,
            cubebot_protocol::network::NetworkPolicy::new(
                vec![],
                vec!["10.96.0.10".parse().unwrap()]
            )
            .unwrap()
        )
        .is_err()
    );
    let layout = Layout::new(
        image,
        image,
        "runtime",
        "https://cubebot.test",
        Some("approved-sandbox".into()),
        cubebot_protocol::network::NetworkPolicy::new(vec![], vec!["10.96.0.10".parse().unwrap()])
            .unwrap(),
    )
    .unwrap();
    assert!(layout.pod("../foreign-claim").is_err());
    assert_eq!(
        layout
            .pod("workspace")
            .unwrap()
            .runtime_class_name
            .as_deref(),
        Some("approved-sandbox")
    );
}

fn verify_container_limits(runtime: &serde_json::Value, tools: &serde_json::Value) {
    assert_eq!(
        runtime["securityContext"]["capabilities"]["drop"],
        json!(["ALL"])
    );
    assert_eq!(tools["securityContext"]["allowPrivilegeEscalation"], false);
    assert_eq!(tools["securityContext"]["readOnlyRootFilesystem"], true);
    assert!(runtime["resources"]["limits"]["memory"].is_string());
    assert!(tools["resources"]["limits"]["memory"].is_string());
}

#[test]
fn text_chat_pod_launches_only_the_runtime_and_retains_private_storage() {
    let image = "registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    let layout = Layout::new(
        image,
        image,
        "cubebot-runtime",
        "https://cubebot.test",
        None,
        cubebot_protocol::network::NetworkPolicy::new(vec![], vec!["10.96.0.10".parse().unwrap()])
            .unwrap(),
    )
    .unwrap();
    let pod = layout.text_chat_pod("workspace").unwrap();
    assert_eq!(pod.containers.len(), 1);
    assert_eq!(pod.containers[0].name, "runtime");
    assert!(
        !pod.init_containers
            .unwrap()
            .iter()
            .any(|container| container.name == "initialize-tools")
    );
    assert!(
        !pod.volumes
            .unwrap()
            .iter()
            .any(|volume| matches!(volume.name.as_str(), "tools-tmp" | "browser-shm"))
    );
}

fn verify_supervisor_capabilities(spec: &serde_json::Value) {
    assert!(spec["containers"][0]["securityContext"]["capabilities"]["add"].is_null());
    assert_eq!(
        spec["containers"][1]["securityContext"]["capabilities"],
        json!({"drop":["ALL"],"add":["KILL","SETUID","SETGID"]})
    );
    assert_eq!(spec["containers"][1]["securityContext"]["runAsUser"], 0);
    assert_eq!(spec["containers"][1]["workingDir"], "/");
}
