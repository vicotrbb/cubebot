//! HTTP and lifecycle work settle before consuming the installation ownership guard.
use crate::agents::kubernetes::{Controller, Error};
use axum::{Router, serve::Listener};
use std::{fmt::Debug, future::Future, io, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    sync::{mpsc, watch},
};
use tokio_rustls::{
    TlsAcceptor,
    rustls::{
        ServerConfig,
        crypto::ring,
        pki_types::{CertificateDer, PrivateKeyDer, pem::PemObject},
    },
    server::TlsStream,
};
use tracing::Instrument;

/// HTTPS on one TCP listener. Handshakes run concurrently, so a stalled client delays no other.
pub struct Tls {
    incoming: mpsc::Receiver<(TlsStream<TcpStream>, SocketAddr)>,
    address: SocketAddr,
}
impl Tls {
    #[tracing::instrument(skip_all)]
    pub fn new(mut listener: TcpListener, chain: &[u8], key: &[u8]) -> io::Result<Self> {
        let chain = CertificateDer::pem_slice_iter(chain)
            .collect::<Result<Vec<_>, _>>()
            .map_err(io::Error::other)?;
        let key = PrivateKeyDer::from_pem_slice(key).map_err(io::Error::other)?;
        let config = ServerConfig::builder_with_provider(Arc::new(ring::default_provider()))
            .with_safe_default_protocol_versions()
            .and_then(|config| config.with_no_client_auth().with_single_cert(chain, key))
            .map_err(io::Error::other)?;
        let acceptor = TlsAcceptor::from(Arc::new(config));
        let address = listener.local_addr()?;
        let (sender, incoming) = mpsc::channel(64);
        tokio::spawn(async move {
            while !sender.is_closed() {
                let (stream, peer) = Listener::accept(&mut listener).await;
                let (acceptor, sender) = (acceptor.clone(), sender.clone());
                let span = tracing::info_span!("tls_handshake", peer = %peer);
                tokio::spawn(
                    async move {
                        let handshake = acceptor.accept(stream);
                        if let Ok(Ok(stream)) =
                            tokio::time::timeout(Duration::from_secs(10), handshake).await
                        {
                            let _ = sender.send((stream, peer)).await;
                        }
                    }
                    .instrument(span),
                );
            }
        });
        Ok(Self { incoming, address })
    }
}
impl Listener for Tls {
    type Io = TlsStream<TcpStream>;
    type Addr = SocketAddr;
    async fn accept(&mut self) -> (Self::Io, Self::Addr) {
        match self.incoming.recv().await {
            Some(connection) => connection,
            None => std::future::pending().await,
        }
    }
    fn local_addr(&self) -> io::Result<SocketAddr> {
        Ok(self.address)
    }
}

pub async fn serve<L>(
    listener: L,
    router: Router,
    mut controller: Option<Controller>,
    shutdown: impl Future<Output = ()>,
) -> Result<(), Error>
where
    L: Listener,
    L::Addr: Debug,
{
    coordinate(listener, router, &mut controller, shutdown).await?;
    if let Some(controller) = controller {
        controller.release().await?;
    }
    Ok(())
}
async fn coordinate<L>(
    listener: L,
    router: Router,
    controller: &mut Option<Controller>,
    shutdown: impl Future<Output = ()>,
) -> Result<(), Error>
where
    L: Listener,
    L::Addr: Debug,
{
    let (stop, receiver) = watch::channel(false);
    let server = axum::serve(listener, router)
        .with_graceful_shutdown(wait_stop(receiver.clone()))
        .into_future();
    let lifecycle = run_controller(controller, receiver);
    tokio::pin!(server, lifecycle, shutdown);
    let (server_done, controller_done, first) = tokio::select! {
        result = &mut server => (true,false,result.map_err(|_| Error::Conflict)),
        result = &mut lifecycle => (false,true,result),
        _ = &mut shutdown => (false,false,Ok(())),
    };
    stop.send_replace(true);
    let (server_result, controller_result) = tokio::join!(
        async {
            if server_done {
                Ok(())
            } else {
                server.await.map_err(|_| Error::Conflict)
            }
        },
        async {
            if controller_done {
                Ok(())
            } else {
                lifecycle.await
            }
        }
    );
    first.and(server_result).and(controller_result)
}
async fn run_controller(
    controller: &mut Option<Controller>,
    receiver: watch::Receiver<bool>,
) -> Result<(), Error> {
    match controller {
        Some(controller) => controller.run(receiver).await,
        None => {
            wait_stop(receiver).await;
            Ok(())
        }
    }
}
async fn wait_stop(mut receiver: watch::Receiver<bool>) {
    if !*receiver.borrow() {
        let _ = receiver.changed().await;
    }
}
