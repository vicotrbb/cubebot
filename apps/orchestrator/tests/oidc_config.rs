use cubebot_orchestrator::identity::oidc::Config;

#[test]
fn rejects_insecure_or_ambiguous_oidc_configuration() {
    for issuer in [
        "http://id.example.com",
        "https://user:pass@id.example.com",
        "https://id.example.com?x=1",
    ] {
        assert!(Config::parse(&serde_json::json!({"issuer": issuer, "client_id": "console", "redirect_uri": "https://cubebot.example.com/api/identity/oidc/callback"}).to_string()).is_err());
    }
    assert!(Config::parse(&serde_json::json!({"issuer": "https://id.example.com", "client_id": "console", "redirect_uri": "https://cubebot.example.com/incorrect"}).to_string()).is_err());
}

#[test]
fn permits_local_fixtures_and_keeps_admission_disabled_by_default() {
    let config = Config::parse(&serde_json::json!({"issuer": "http://127.0.0.1:9999", "client_id": "console", "redirect_uri": "http://127.0.0.1:8080/api/identity/oidc/callback"}).to_string()).unwrap();
    assert!(config.admit_subjects.is_empty());
    assert!(config.admit_email_domains.is_empty());
}
