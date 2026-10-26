use cubebot_protocol::network::NetworkPolicy;
use serde_json::json;

#[test]
fn network_contract_rejects_ambiguous_or_excessive_topology() {
    for input in [
        json!({"infrastructure":["10.0.0.1/8"],"dns":[]}),
        json!({"infrastructure":["::ffff:8.8.4.4/128"],"dns":[]}),
        json!({"infrastructure":vec!["10.0.0.0/8";129],"dns":[]}),
        json!({"infrastructure":[],"dns":vec!["1.1.1.1";4]}),
        json!({"infrastructure":[],"dns":[],"allow_all":true}),
    ] {
        assert!(serde_json::from_value::<NetworkPolicy>(input).is_err());
    }
    let policy: NetworkPolicy = serde_json::from_value(
        json!({"infrastructure":["8.8.4.4/32"],"dns":["::ffff:10.96.0.10"]}),
    )
    .unwrap();
    let encoded = serde_json::to_value(&policy).unwrap();
    assert_eq!(encoded["dns"], json!(["10.96.0.10"]));
    assert_eq!(encoded["infrastructure"], json!(["8.8.4.4/32"]));
}
