use cubebot_orchestrator::agents::kubernetes::ControllerConfig;
use serde_json::{Value, json};
fn valid() -> Value {
    let image = "registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    json!({"connection":{"kind":"in_cluster"},"runtime_image":image,"tools_image":image,"runtime_service_account":"cubebot-runtime","orchestrator_origin":"https://cubebot.test"})
}
#[test]
fn controller_configuration_rejects_unsafe_workload_fields_and_implicit_connections() {
    assert!(ControllerConfig::parse(&valid().to_string()).is_ok());
    for (key, value) in [
        ("runtime_image", json!("registry.test/latest")),
        ("runtime_service_account", json!("default")),
        ("orchestrator_origin", json!("http://cubebot.test")),
        ("connection", json!({"kind":"infer"})),
        (
            "connection",
            json!({"kind":"kubeconfig","path":"/trusted/config","context":""}),
        ),
        ("unexpected", json!(true)),
    ] {
        let mut input = valid();
        input[key] = value;
        assert!(
            ControllerConfig::parse(&input.to_string()).is_err(),
            "{key}"
        );
    }
    assert!(ControllerConfig::parse(&" ".repeat(65537)).is_err());
}
