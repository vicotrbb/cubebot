use axum::{Router, routing::get, serve::Listener};
use cubebot_orchestrator::operations::server::Tls;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_rustls::{
    TlsConnector,
    rustls::{
        ClientConfig, RootCertStore,
        crypto::ring,
        pki_types::{CertificateDer, ServerName, pem::PemObject},
    },
};

// The agent's public localhost fixtures; their README forbids any installation use.
const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../agent/tests/fixtures/control-tls"
);

#[tokio::test]
async fn tls_listener_serves_the_router_past_a_stalled_handshake() {
    let read = |name: &str| std::fs::read(format!("{FIXTURES}/{name}")).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let listener = Tls::new(listener, &read("server.pem"), &read("server.key")).unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(
        axum::serve(listener, Router::new().route("/ok", get(|| async { "ok" }))).into_future(),
    );
    let _stalled = tokio::net::TcpStream::connect(address).await.unwrap();

    let mut roots = RootCertStore::empty();
    roots
        .add(CertificateDer::from_pem_slice(&read("ca.pem")).unwrap())
        .unwrap();
    let client = ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    let tcp = tokio::net::TcpStream::connect(address).await.unwrap();
    let name = ServerName::try_from("localhost").unwrap();
    let mut stream = TlsConnector::from(Arc::new(client))
        .connect(name, tcp)
        .await
        .unwrap();
    stream
        .write_all(b"GET /ok HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    assert!(
        response.starts_with("HTTP/1.1 200") && response.ends_with("ok"),
        "{response}"
    );
}
