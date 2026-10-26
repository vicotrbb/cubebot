use cubebot_agent::control::{Config, Error, RuntimeClient};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        ServerConfig,
        pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    },
};

const CA: &[u8] = include_bytes!("fixtures/control-tls/ca.pem");

async fn exchange(trusted: bool, hostname: &str) -> Result<(), Error> {
    let certificate =
        CertificateDer::from_pem_slice(include_bytes!("fixtures/control-tls/server.pem")).unwrap();
    let key =
        PrivateKeyDer::from_pem_slice(include_bytes!("fixtures/control-tls/server.key")).unwrap();
    let server = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![certificate], key)
        .unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let Ok(mut stream) = TlsAcceptor::from(Arc::new(server)).accept(stream).await else {
            return;
        };
        let mut input = [0; 4096];
        let count = stream.read(&mut input).await.unwrap();
        assert!(
            std::str::from_utf8(&input[..count])
                .unwrap()
                .starts_with("POST /api/runtime/bootstrap ")
        );
        let body = format!(
            r#"{{"token":"cb_runtime_{}","agent":"00000000-0000-0000-0000-000000000001","attempt":"00000000-0000-0000-0000-000000000002","epoch":1,"version":1,"expires_in_seconds":120}}"#,
            "a".repeat(64)
        );
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            body.len(),
            body
        );
        stream.write_all(response.as_bytes()).await.unwrap();
        stream.shutdown().await.unwrap();
    });
    let token = std::env::temp_dir().join(format!("cubebot-tls-{}", uuid::Uuid::new_v4()));
    std::fs::write(&token, "test-only-projected-token").unwrap();
    let mut config = Config::new(&format!("https://{hostname}:{port}"), token.clone()).unwrap();
    if trusted {
        config = config.with_root_certificate(CA).unwrap();
    }
    let outcome = tokio::time::timeout(
        Duration::from_secs(15),
        RuntimeClient::new(config).unwrap().bootstrap(),
    )
    .await
    .unwrap();
    task.await.unwrap();
    std::fs::remove_file(token).unwrap();
    outcome
}

#[tokio::test]
async fn selected_ca_preserves_certificate_and_hostname_verification() {
    assert!(exchange(true, "localhost").await.is_ok());
    assert!(matches!(
        exchange(false, "localhost").await,
        Err(Error::Unavailable)
    ));
    assert!(matches!(
        exchange(true, "127.0.0.1").await,
        Err(Error::Unavailable)
    ));
}

#[test]
fn malformed_or_oversized_ca_is_rejected() {
    for pem in [b"not a certificate".as_slice(), &vec![b'x'; 65537]] {
        assert!(
            Config::new("https://localhost", "/unused".into())
                .unwrap()
                .with_root_certificate(pem)
                .is_err()
        );
    }
}

#[tokio::test]
async fn configured_ca_file_is_required_and_bounded_at_process_startup() {
    let path = std::env::temp_dir().join(format!("cubebot-ca-file-{}", uuid::Uuid::new_v4()));
    for contents in [None, Some(Vec::new()), Some(vec![b'x'; 65537])] {
        if let Some(contents) = contents {
            std::fs::write(&path, contents).unwrap();
        }
        let output = tokio::time::timeout(
            Duration::from_secs(10),
            tokio::process::Command::new(env!("CARGO_BIN_EXE_cubebot-agent"))
                .env_clear()
                .env("CUBEBOT_ORCHESTRATOR_ORIGIN", "https://localhost")
                .env("CUBEBOT_BOOTSTRAP_TOKEN_FILE", "/unused")
                .env("CUBEBOT_CONTROL_CA_FILE", &path)
                .kill_on_drop(true)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(!output.status.success());
        assert!(
            String::from_utf8(output.stderr)
                .unwrap()
                .contains("invalid runtime configuration")
        );
    }
    std::fs::remove_file(path).unwrap();
}
