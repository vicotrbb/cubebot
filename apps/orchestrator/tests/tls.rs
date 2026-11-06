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

const FIXTURES: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../agent/tests/fixtures/control-tls"
);

fn client() -> TlsConnector {
    let mut roots = RootCertStore::empty();
    roots
        .add(
            CertificateDer::from_pem_slice(&std::fs::read(format!("{FIXTURES}/ca.pem")).unwrap())
                .unwrap(),
        )
        .unwrap();
    let config = ClientConfig::builder_with_provider(Arc::new(ring::default_provider()))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_root_certificates(roots)
        .with_no_client_auth();
    TlsConnector::from(Arc::new(config))
}

async fn health<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin>(mut stream: S) -> String {
    stream
        .write_all(b"GET /api/health/live HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    response
}

#[tokio::test]
async fn serve_answers_on_the_public_and_control_listeners_and_stops_both() {
    let read = |name: &str| std::fs::read(format!("{FIXTURES}/{name}")).unwrap();
    let public = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let public_address = public.local_addr().unwrap();
    let control = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let control = Tls::new(control, &read("server.pem"), &read("server.key")).unwrap();
    let control_address = control.local_addr().unwrap();
    let (stop, stopped) = tokio::sync::oneshot::channel::<()>();
    let serving = tokio::spawn(cubebot_orchestrator::operations::server::serve(
        public,
        Some(control),
        cubebot_orchestrator::api::router(),
        None,
        async {
            let _ = stopped.await;
        },
    ));

    let plain = health(
        tokio::net::TcpStream::connect(public_address)
            .await
            .unwrap(),
    )
    .await;
    assert!(plain.starts_with("HTTP/1.1 200"), "{plain}");
    let tcp = tokio::net::TcpStream::connect(control_address)
        .await
        .unwrap();
    let name = ServerName::try_from("localhost").unwrap();
    let secure = health(client().connect(name, tcp).await.unwrap()).await;
    assert!(secure.starts_with("HTTP/1.1 200"), "{secure}");

    stop.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), serving)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert!(
        tokio::net::TcpStream::connect(public_address)
            .await
            .is_err()
    );
}

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
