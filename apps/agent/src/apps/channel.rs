use super::Apps;
use crate::control::{Channel, RuntimeClient};
use cubebot_protocol::apps::{AppNotice, AppReport, Frame, LOCAL_PORT, decode, encode};
use futures_util::{SinkExt, StreamExt};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    sync::mpsc,
    time::Instant,
};
use tokio_tungstenite::tungstenite::{Error, Message};

const RECONNECT: Duration = Duration::from_secs(5);
const SILENCE: Duration = Duration::from_secs(15);
const STALL: Duration = Duration::from_secs(10);
const CHUNK: usize = 16 * 1024;
const QUEUED: usize = 8;
const OUTGOING: usize = 64;

enum Out {
    Data(u32, Vec<u8>),
    Close(u32),
}

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
    let mut streams = HashMap::new();
    let mut heard = Instant::now();
    loop {
        let going = tokio::select! {
            message = channel.next() => {
                heard = Instant::now();
                incoming(apps, &mut streams, &out, port, message).await
            }
            Some(report) = reports.recv() => {
                let text = serde_json::to_string(&report).unwrap_or_default();
                channel.send(Message::Text(text.into())).await.is_ok()
            }
            Some(item) = outgoing.recv() => {
                let frame = match item {
                    Out::Data(stream, data) => encode(&Frame::Data(stream, &data)),
                    Out::Close(stream) => {
                        streams.remove(&stream);
                        encode(&Frame::Close(stream))
                    }
                };
                channel.send(Message::Binary(frame.into())).await.is_ok()
            }
            () = tokio::time::sleep_until(heard + SILENCE) => false,
        };
        if !going {
            return;
        }
    }
}

async fn incoming(
    apps: &Arc<Apps>,
    streams: &mut HashMap<u32, mpsc::Sender<Vec<u8>>>,
    out: &mpsc::Sender<Out>,
    port: u16,
    message: Option<Result<Message, Error>>,
) -> bool {
    match message {
        Some(Ok(Message::Binary(bytes))) => {
            deliver(streams, out, port, &bytes).await;
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

async fn deliver(
    streams: &mut HashMap<u32, mpsc::Sender<Vec<u8>>>,
    out: &mpsc::Sender<Out>,
    port: u16,
    bytes: &[u8],
) {
    match decode(bytes) {
        Some(Frame::Open(stream)) => {
            let (inbound, received) = mpsc::channel(QUEUED);
            streams.insert(stream, inbound);
            tokio::spawn(pump(stream, port, received, out.clone()));
        }
        Some(Frame::Data(stream, data)) => {
            let Some(sender) = streams.get(&stream) else {
                return;
            };
            let sent = tokio::time::timeout(STALL, sender.send(data.to_vec())).await;
            if !matches!(sent, Ok(Ok(()))) {
                streams.remove(&stream);
            }
        }
        Some(Frame::Close(stream)) => {
            streams.remove(&stream);
        }
        None => {}
    }
}

async fn pump(
    stream: u32,
    port: u16,
    mut inbound: mpsc::Receiver<Vec<u8>>,
    out: mpsc::Sender<Out>,
) {
    if let Ok(local) = TcpStream::connect(("127.0.0.1", port)).await {
        let (mut reader, mut writer) = local.into_split();
        let up = async {
            let mut buffer = vec![0; CHUNK];
            loop {
                let read = match reader.read(&mut buffer).await {
                    Ok(0) | Err(_) => break,
                    Ok(read) => read,
                };
                if out
                    .send(Out::Data(stream, buffer[..read].to_vec()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        };
        let down = async {
            while let Some(data) = inbound.recv().await {
                if writer.write_all(&data).await.is_err() {
                    break;
                }
            }
        };
        tokio::select! {
            () = up => {}
            () = down => {}
        }
    }
    let _ = out.send(Out::Close(stream)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
