use super::Apps;
use crate::control::{Channel, RuntimeClient};
use cubebot_protocol::apps::{AppNotice, AppReport, Frame, LOCAL_PORT, decode, encode};
use futures_util::{SinkExt, StreamExt};
use std::{collections::HashMap, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::{Semaphore, mpsc},
    time::Instant,
};
use tokio_tungstenite::tungstenite::{Error, Message};

const RECONNECT: Duration = Duration::from_secs(5);
const SILENCE: Duration = Duration::from_secs(15);
const CHUNK: usize = 16 * 1024;
const WINDOW: usize = 256 * 1024;
const GRANT: usize = 64 * 1024;
const FRAME_COST: usize = 64;
const CHUNK_CREDIT: u32 = 16 * 1024 + 64;
const OUTGOING: usize = 64;

enum Out {
    Data(u32, Vec<u8>),
    Credit(u32, u32),
    Close(u32),
}

struct Lane {
    sender: mpsc::Sender<Vec<u8>>,
    credit: Arc<Semaphore>,
    owed: usize,
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
                incoming(apps, &mut streams, &out, port, message)
            }
            Some(report) = reports.recv() => {
                let text = serde_json::to_string(&report).unwrap_or_default();
                channel.send(Message::Text(text.into())).await.is_ok()
            }
            Some(item) = outgoing.recv() => {
                let frame = outbound(&mut streams, item);
                channel.send(Message::Binary(frame.into())).await.is_ok()
            }
            () = tokio::time::sleep_until(heard + SILENCE) => false,
        };
        if !going {
            return;
        }
    }
}

fn outbound(streams: &mut HashMap<u32, Lane>, item: Out) -> Vec<u8> {
    match item {
        Out::Data(stream, data) => encode(&Frame::Data(stream, &data)),
        Out::Credit(stream, bytes) => {
            if let Some(lane) = streams.get_mut(&stream) {
                lane.owed = lane
                    .owed
                    .saturating_sub(usize::try_from(bytes).unwrap_or(usize::MAX));
            }
            encode(&Frame::Credit(stream, bytes))
        }
        Out::Close(stream) => {
            streams.remove(&stream);
            encode(&Frame::Close(stream))
        }
    }
}

fn incoming(
    apps: &Arc<Apps>,
    streams: &mut HashMap<u32, Lane>,
    out: &mpsc::Sender<Out>,
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

fn deliver(streams: &mut HashMap<u32, Lane>, out: &mpsc::Sender<Out>, port: u16, bytes: &[u8]) {
    let (stream, kept) = match decode(bytes) {
        Some(Frame::Open(stream)) => {
            let (sender, received) = mpsc::channel(WINDOW / FRAME_COST);
            let credit = Arc::new(Semaphore::new(WINDOW));
            let lane = Lane {
                sender,
                credit: credit.clone(),
                owed: 0,
            };
            streams.insert(stream, lane);
            tokio::spawn(pump(stream, port, received, credit, out.clone()));
            return;
        }
        Some(Frame::Data(stream, data)) => (stream, accepted(streams.get_mut(&stream), data)),
        Some(Frame::Credit(stream, bytes)) => (stream, credited(streams.get(&stream), bytes)),
        Some(Frame::Close(stream)) => (stream, false),
        None => return,
    };
    if !kept {
        streams.remove(&stream);
    }
}

fn accepted(lane: Option<&mut Lane>, data: &[u8]) -> bool {
    let Some(lane) = lane else {
        return true;
    };
    lane.owed += data.len() + FRAME_COST;
    lane.owed <= WINDOW && lane.sender.try_send(data.to_vec()).is_ok()
}

fn credited(lane: Option<&Lane>, bytes: u32) -> bool {
    let Some(lane) = lane else {
        return true;
    };
    let bytes = usize::try_from(bytes).unwrap_or(usize::MAX);
    if lane.credit.available_permits().saturating_add(bytes) > WINDOW {
        return false;
    }
    lane.credit.add_permits(bytes);
    true
}

async fn pump(
    stream: u32,
    port: u16,
    inbound: mpsc::Receiver<Vec<u8>>,
    credit: Arc<Semaphore>,
    out: mpsc::Sender<Out>,
) {
    if let Ok(local) = TcpStream::connect(("127.0.0.1", port)).await {
        let (reader, writer) = local.into_split();
        tokio::select! {
            () = send(stream, reader, &credit, &out) => {}
            () = receive(stream, writer, inbound, &out) => {}
        }
    }
    let _ = out.send(Out::Close(stream)).await;
}

async fn send(
    stream: u32,
    mut reader: impl AsyncRead + Unpin,
    credit: &Semaphore,
    out: &mpsc::Sender<Out>,
) {
    let mut buffer = vec![0; CHUNK];
    loop {
        let Ok(permit) = credit.acquire_many(CHUNK_CREDIT).await else {
            return;
        };
        permit.forget();
        let read = match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(read) => read,
        };
        credit.add_permits(CHUNK - read);
        if out
            .send(Out::Data(stream, buffer[..read].to_vec()))
            .await
            .is_err()
        {
            return;
        }
    }
}

async fn receive(
    stream: u32,
    mut writer: impl AsyncWrite + Unpin,
    mut inbound: mpsc::Receiver<Vec<u8>>,
    out: &mpsc::Sender<Out>,
) {
    let mut written = 0;
    while let Some(data) = inbound.recv().await {
        if writer.write_all(&data).await.is_err() {
            return;
        }
        written += data.len() + FRAME_COST;
        if written >= GRANT {
            let bytes = u32::try_from(written).unwrap_or(u32::MAX);
            if out.send(Out::Credit(stream, bytes)).await.is_err() {
                return;
            }
            written = 0;
        }
    }
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
