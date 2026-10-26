use cubebot_orchestrator::agents::kubernetes::Layout;
use serde_json::json;

#[test]
fn compiled_layout_separates_identity_journal_and_tool_mounts() {
    let layout = Layout::new("registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa", "registry.test/tools@sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", "cubebot-runtime", "https://cubebot.test", None).unwrap();
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
        spec["initContainers"][0]["args"],
        json!(["--initialize-storage", "runtime"])
    );
    assert_eq!(
        spec["initContainers"][1]["args"],
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
            None
        )
        .is_err()
    );
    assert!(Layout::new(image, image, "default", "https://cubebot.test", None).is_err());
    assert!(Layout::new(image, image, "runtime", "http://cubebot.test", None).is_err());
    assert!(
        Layout::new(
            image,
            image,
            "runtime",
            "https://user:secret@cubebot.test",
            None
        )
        .is_err()
    );
    let layout = Layout::new(
        image,
        image,
        "runtime",
        "https://cubebot.test",
        Some("approved-sandbox".into()),
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
