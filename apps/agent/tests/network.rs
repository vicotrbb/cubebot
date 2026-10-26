use cubebot_agent::execution::network::Policy;

#[test]
fn setup_command_validates_before_requesting_privilege() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let mut child = Command::new(env!("CARGO_BIN_EXE_cubebot-agent"))
        .arg("--initialize-network")
        .arg(r#"{"infrastructure":["inject; drop"],"dns":[]}"#)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("network setup waited for open stdin");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let output = child.wait_with_output().unwrap();
    assert!(!output.status.success());
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("invalid infrastructure prefix")
    );
    assert!(output.stdout.is_empty());
}

#[test]
fn untrusted_configuration_cannot_inject_rules_or_omit_topology() {
    for input in [
        r#"{"dns":[]}"#,
        r#"{"infrastructure":[],"dns":[],"allow_all":true}"#,
        r#"{"infrastructure":["10.0.0.0/8; flush ruleset"],"dns":[]}"#,
        r#"{"infrastructure":[],"dns":["127.0.0.1; accept"]}"#,
        r#"{"infrastructure":["10.0.0.1/8"],"dns":[]}"#,
    ] {
        assert!(Policy::parse(input).is_err(), "{input}");
    }
    assert!(Policy::parse(&" ".repeat(16385)).is_err());
}
