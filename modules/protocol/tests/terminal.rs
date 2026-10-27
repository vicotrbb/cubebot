use cubebot_protocol::terminal::Invocation;

#[test]
fn rejects_unbounded_or_unattributed_execution() {
    let valid = serde_json::json!({
        "id":"00000000-0000-4000-8000-000000000001",
        "authority":"00000000-0000-4000-8000-000000000002",
        "command":"printf hello", "seconds":5
    });
    let request: Invocation = serde_json::from_value(valid.clone()).unwrap();
    assert!(request.validate().is_ok());
    for (field, value) in [
        ("seconds", serde_json::json!(0)),
        ("seconds", serde_json::json!(31)),
        ("command", serde_json::json!("")),
        ("command", serde_json::json!("\0")),
        (
            "id",
            serde_json::json!("00000000-0000-0000-0000-000000000000"),
        ),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = value;
        let request: Invocation = serde_json::from_value(invalid).unwrap();
        assert!(request.validate().is_err());
    }
}
