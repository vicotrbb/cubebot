use cubebot_protocol::terminal::Invocation;
use uuid::Uuid;

#[test]
fn rejects_unattributed_or_invalid_execution() {
    let valid = serde_json::json!({
        "id":"00000000-0000-4000-8000-000000000001",
        "authority":"00000000-0000-4000-8000-000000000002",
        "command":"printf hello"
    });
    let request: Invocation = serde_json::from_value(valid.clone()).unwrap();
    assert!(request.validate().is_ok());
    for (field, value) in [
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

#[test]
fn terminal_commands_do_not_require_a_duration_budget() {
    let request = serde_json::json!({
        "id":"00000000-0000-4000-8000-000000000001",
        "authority":"00000000-0000-4000-8000-000000000002",
        "command":"sleep 90"
    });
    assert!(serde_json::from_value::<Invocation>(request).is_ok());
}

#[test]
fn retained_duration_fields_remain_readable_without_becoming_execution_limits() {
    let value = serde_json::json!({"id":Uuid::new_v4(),"authority":Uuid::new_v4(),"command":"true","seconds":30});
    let request: Invocation = serde_json::from_value(value.clone()).unwrap();
    assert!(
        serde_json::to_value(request)
            .unwrap()
            .get("seconds")
            .is_none()
    );
    let mut invalid = value;
    invalid["other_authority"] = serde_json::json!("not allowed");
    assert!(serde_json::from_value::<Invocation>(invalid).is_err());
}
