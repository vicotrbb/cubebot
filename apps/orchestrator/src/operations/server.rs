//! HTTP and lifecycle work settle before consuming the installation ownership guard.
use crate::agents::kubernetes::{Controller, Error};
use axum::Router;
use std::future::Future;
use tokio::{net::TcpListener, sync::watch};

pub async fn serve(
    listener: TcpListener,
    router: Router,
    mut controller: Option<Controller>,
    shutdown: impl Future<Output = ()>,
) -> Result<(), Error> {
    coordinate(listener, router, &mut controller, shutdown).await?;
    if let Some(controller) = controller {
        controller.release().await?;
    }
    Ok(())
}
async fn coordinate(
    listener: TcpListener,
    router: Router,
    controller: &mut Option<Controller>,
    shutdown: impl Future<Output = ()>,
) -> Result<(), Error> {
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
