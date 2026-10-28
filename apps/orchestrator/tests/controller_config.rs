use cubebot_orchestrator::agents::kubernetes::ControllerConfig;
use serde_json::{Value, json};
fn valid() -> Value {
    let image = "registry.test/agent@sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    json!({"connection":{"kind":"in_cluster"},"sandbox_image":image,"backend":"process","runtime_service_account":"cubebot-runtime","orchestrator_origin":"https://cubebot.test","network":{"dns":["10.96.0.10"]}})
}
#[test]
fn controller_accepts_only_explicit_valid_ca_config_map_names() {
    let mut config = valid();
    config["control_ca_config_map"] = json!("control-ca");
    assert!(ControllerConfig::parse(&config.to_string()).is_ok());
    config["control_ca_config_map"] = json!("kube-root-ca.crt");
    assert!(ControllerConfig::parse(&config.to_string()).is_ok());
    for name in [
        "",
        "../ca",
        "control/ca",
        "UPPER",
        "ca..crt",
        "-ca.crt",
        "ca-.crt",
        &"a".repeat(254),
    ] {
        config["control_ca_config_map"] = json!(name);
        assert!(ControllerConfig::parse(&config.to_string()).is_err());
    }
}
#[test]
fn controller_configuration_rejects_unsafe_workload_fields_and_implicit_connections() {
    assert!(ControllerConfig::parse(&valid().to_string()).is_ok());
    for (key, value) in [
        ("sandbox_image", json!("registry.test/latest")),
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

#[test]
fn controller_requires_explicit_dns_and_a_known_backend() {
    let mut input = valid();
    input.as_object_mut().unwrap().remove("network");
    assert!(ControllerConfig::parse(&input.to_string()).is_err());
    input["network"] = json!({"dns":[]});
    assert!(ControllerConfig::parse(&input.to_string()).is_err());
    input["network"] = json!({"dns":["10.96.0.10"],"infrastructure":[]});
    assert!(ControllerConfig::parse(&input.to_string()).is_err());
    input["network"] = json!({"dns":["10.96.0.10"]});
    input["backend"] = json!("kvm");
    assert!(ControllerConfig::parse(&input.to_string()).is_ok());
    input["backend"] = json!("container");
    assert!(ControllerConfig::parse(&input.to_string()).is_err());
}
