use super::Apps;
use crate::control::{Channel, RuntimeClient};
use cubebot_protocol::{
    apps::{AppNotice, AppReport, Frame, LOCAL_PORT, decode},
    lanes::{self, Lanes, Piece, Pipe},
};
use futures_util::{SinkExt, StreamExt};
use std::{sync::Arc, time::Duration};
use tokio::{net::TcpStream, sync::mpsc, time::Instant};
use tokio_tungstenite::tungstenite::{Error, Message};

const RECONNECT: Duration = Duration::from_secs(5);
const SILENCE: Duration = Duration::from_secs(15);
const OUTGOING: usize = 64;

pub(super) async fn run(apps: Arc<Apps>, client: RuntimeClient) {
    let Some(mut reports) = apps.reports() else {
        return;
    };
    loop {
        match client.open("api/runtime/apps/channel").await {
            Ok(channel) => link(&apps, channel, &mut reports, LOCAL_PORT).await,
            Err(error) => eprintln!("App channel waiting: {error}"),
        }
        tokio::time::sleep(RECONNECT).await;
    }
}

fn refresh(apps: &Arc<Apps>) {
    let apps = apps.clone();
    tokio::spawn(async move { apps.refresh().await });
}

async fn link(
    apps: &Arc<Apps>,
    mut channel: Channel,
    reports: &mut mpsc::Receiver<AppReport>,
    port: u16,
) {
    refresh(apps);
    let (out, mut outgoing) = mpsc::channel(OUTGOING);
    let mut streams = Lanes::default();
    let mut heard = Instant::now();
    loop {
        let going = tokio::select! {
            message = channel.next() => {
                heard = Instant::now();
                incoming(apps, &mut streams, &out, port, message)
            }
            Some(report) = reports.recv() => {
                let text = serde_json::to_string(&report).unwrap_or_default();
                sent(&mut channel, Message::Text(text.into())).await
            }
            Some(piece) = outgoing.recv() => {
                let frame = streams.outgoing(&piece);
                sent(&mut channel, Message::Binary(frame.into())).await
            }
            () = tokio::time::sleep_until(heard + SILENCE) => false,
        };
        if !going {
            return;
        }
    }
}

async fn sent(channel: &mut Channel, message: Message) -> bool {
    tokio::time::timeout(SILENCE, channel.send(message))
        .await
        .is_ok_and(|sent| sent.is_ok())
}

fn incoming(
    apps: &Arc<Apps>,
    streams: &mut Lanes,
    out: &mpsc::Sender<Piece>,
    port: u16,
    message: Option<Result<Message, Error>>,
) -> bool {
    match message {
        Some(Ok(Message::Binary(bytes))) => {
            deliver(streams, out, port, &bytes);
            true
        }
        Some(Ok(Message::Text(text))) => {
            if serde_json::from_str::<AppNotice>(text.as_str()).is_ok() {
                refresh(apps);
            }
            true
        }
        Some(Ok(Message::Ping(_) | Message::Pong(_))) => true,
        _ => false,
    }
}

fn deliver(streams: &mut Lanes, out: &mpsc::Sender<Piece>, port: u16, bytes: &[u8]) {
    match decode(bytes) {
        Some(Frame::Open(stream)) => {
            let (lane, pipe) = lanes::lane();
            streams.insert(stream, lane);
            tokio::spawn(pump(stream, port, pipe, out.clone()));
        }
        Some(frame) => streams.receive(&frame),
        None => {}
    }
}

async fn pump(stream: u32, port: u16, pipe: Pipe, out: mpsc::Sender<Piece>) {
    match TcpStream::connect(("127.0.0.1", port)).await {
        Ok(local) => lanes::pump(stream, local, pipe, out).await,
        Err(_) => {
            let _ = out.send(Piece::Close(stream)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cubebot_protocol::{
        apps::encode,
        lanes::{CHUNK, FRAME_COST, WINDOW},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    #[tokio::test]
    async fn streams_reach_the_local_server_and_reports_flow_out() {
        let local = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = local.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = local.accept().await.unwrap();
            let mut asked = vec![0; 4];
            socket.read_exact(&mut asked).await.unwrap();
            assert_eq!(asked, b"ping");
            socket.write_all(b"pong").await.unwrap();
        });
        let orchestrator = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = orchestrator.local_addr().unwrap();
        let far = tokio::spawn(async move {
            let (socket, _) = orchestrator.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            socket
                .send(Message::Binary(encode(&Frame::Open(7)).into()))
                .await
                .unwrap();
            socket
                .send(Message::Binary(encode(&Frame::Data(7, b"ping")).into()))
                .await
                .unwrap();
            let mut seen = Vec::new();
            while seen.len() < 3 {
                match socket.next().await.unwrap().unwrap() {
                    Message::Binary(bytes) => seen.push(format!("{:?}", decode(&bytes).unwrap())),
                    Message::Text(text) => seen.push(text.to_string()),
                    _ => {}
                }
            }
            seen
        });
        let (channel, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let apps = Arc::new(Apps::new(None, None));
        let mut reports = apps.reports().unwrap();
        apps.reports
            .try_send(AppReport::Called {
                slug: "budget".into(),
                tool: "whoop__recovery".into(),
                viewer: None,
                ok: true,
            })
            .unwrap();
        tokio::spawn(async move { link(&apps, channel, &mut reports, port).await });
        let mut seen = tokio::time::timeout(Duration::from_secs(5), far)
            .await
            .unwrap()
            .unwrap();
        seen.sort();
        assert_eq!(
            seen,
            [
                "Close(7)",
                "Data(7, [112, 111, 110, 103])",
                r#"{"type":"called","slug":"budget","tool":"whoop__recovery","viewer":null,"ok":true}"#,
            ]
        );
    }

    #[tokio::test]
    async fn the_agent_sends_only_within_the_granted_credit() {
        let local = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = local.local_addr().unwrap().port();
        tokio::spawn(async move {
            let (mut socket, _) = local.accept().await.unwrap();
            let _ = socket.write_all(&vec![b'x'; 1024 * 1024]).await;
        });
        let orchestrator = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = orchestrator.local_addr().unwrap();
        let far = tokio::spawn(async move {
            let (socket, _) = orchestrator.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            socket
                .send(Message::Binary(encode(&Frame::Open(3)).into()))
                .await
                .unwrap();
            let mut spent = 0;
            let quiet = Duration::from_millis(300);
            while let Ok(Some(Ok(Message::Binary(bytes)))) =
                tokio::time::timeout(quiet, socket.next()).await
            {
                if let Some(Frame::Data(3, data)) = decode(&bytes) {
                    spent += data.len() + FRAME_COST;
                }
            }
            let first = spent;
            let grant = u32::try_from(first).unwrap();
            socket
                .send(Message::Binary(encode(&Frame::Credit(3, grant)).into()))
                .await
                .unwrap();
            while let Ok(Some(Ok(Message::Binary(bytes)))) =
                tokio::time::timeout(quiet, socket.next()).await
            {
                if let Some(Frame::Data(3, data)) = decode(&bytes) {
                    spent += data.len() + FRAME_COST;
                }
            }
            (first, spent)
        });
        let (channel, _) = tokio_tungstenite::connect_async(format!("ws://{address}"))
            .await
            .unwrap();
        let apps = Arc::new(Apps::new(None, None));
        let mut reports = apps.reports().unwrap();
        tokio::spawn(async move { link(&apps, channel, &mut reports, port).await });
        let (first, total) = far.await.unwrap();
        assert!(
            first > WINDOW - CHUNK - FRAME_COST && first <= WINDOW,
            "{first}"
        );
        assert!(total > first && total <= 2 * WINDOW, "{total}");
    }
}
