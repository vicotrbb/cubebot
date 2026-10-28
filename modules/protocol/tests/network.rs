use cubebot_protocol::network::NetworkPolicy;
use serde_json::json;

#[test]
fn network_contract_rejects_ambiguous_or_excessive_configuration() {
    for input in [
        json!({"dns":vec!["1.1.1.1";4]}),
        json!({"dns":[],"allow_all":true}),
        json!({"infrastructure":[],"dns":[]}),
    ] {
        assert!(serde_json::from_value::<NetworkPolicy>(input).is_err());
    }
    let policy: NetworkPolicy =
        serde_json::from_value(json!({"dns":["::ffff:10.96.0.10"]})).unwrap();
    let encoded = serde_json::to_value(&policy).unwrap();
    assert_eq!(encoded["dns"], json!(["10.96.0.10"]));
}
