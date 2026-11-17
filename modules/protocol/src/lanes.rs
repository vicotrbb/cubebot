use crate::apps::{Frame, encode};
use std::{collections::HashMap, sync::Arc};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    sync::{Semaphore, mpsc},
};

pub const CHUNK: usize = 16 * 1024;
pub const WINDOW: usize = 256 * 1024;
pub const GRANT: usize = 64 * 1024;
pub const FRAME_COST: usize = 64;
const CHUNK_CREDIT: u32 = 16 * 1024 + 64;

#[derive(Debug, PartialEq, Eq)]
pub enum Piece {
    Data(u32, Vec<u8>),
    Credit(u32, u32),
    Close(u32),
}

pub struct Lane {
    sender: mpsc::Sender<Vec<u8>>,
    credit: Arc<Semaphore>,
    owed: usize,
}

pub struct Pipe {
    inbound: mpsc::Receiver<Vec<u8>>,
    credit: Arc<Semaphore>,
}

pub fn lane() -> (Lane, Pipe) {
    let (sender, inbound) = mpsc::channel(WINDOW / FRAME_COST);
    let credit = Arc::new(Semaphore::new(WINDOW));
    let lane = Lane {
        sender,
        credit: credit.clone(),
        owed: 0,
    };
    (lane, Pipe { inbound, credit })
}

#[derive(Default)]
pub struct Lanes(HashMap<u32, Lane>);

impl Lanes {
    pub fn insert(&mut self, stream: u32, lane: Lane) {
        self.0.insert(stream, lane);
    }

    pub fn receive(&mut self, frame: &Frame<'_>) {
        let (stream, kept) = match *frame {
            Frame::Data(stream, data) => (stream, accepted(self.0.get_mut(&stream), data)),
            Frame::Credit(stream, bytes) => (stream, credited(self.0.get(&stream), bytes)),
            Frame::Close(stream) => (stream, false),
            Frame::Open(_) => return,
        };
        if !kept {
            self.0.remove(&stream);
        }
    }

    pub fn outgoing(&mut self, piece: &Piece) -> Vec<u8> {
        match piece {
            Piece::Data(stream, data) => encode(&Frame::Data(*stream, data)),
            Piece::Credit(stream, bytes) => {
                if let Some(lane) = self.0.get_mut(stream) {
                    let bytes = usize::try_from(*bytes).unwrap_or(usize::MAX);
                    lane.owed = lane.owed.saturating_sub(bytes);
                }
                encode(&Frame::Credit(*stream, *bytes))
            }
            Piece::Close(stream) => {
                self.0.remove(stream);
                encode(&Frame::Close(*stream))
            }
        }
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

pub async fn pump<T: From<Piece>>(
    stream: u32,
    io: impl AsyncRead + AsyncWrite,
    pipe: Pipe,
    out: mpsc::Sender<T>,
) {
    let (reader, writer) = tokio::io::split(io);
    tokio::select! {
        () = send(stream, reader, &pipe.credit, &out) => {}
        () = receive(stream, writer, pipe.inbound, &out) => {}
    }
    let _ = out.send(Piece::Close(stream).into()).await;
}

async fn send<T: From<Piece>>(
    stream: u32,
    mut reader: impl AsyncRead + Unpin,
    credit: &Semaphore,
    out: &mpsc::Sender<T>,
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
        let piece = Piece::Data(stream, buffer[..read].to_vec());
        if out.send(piece.into()).await.is_err() {
            return;
        }
    }
}

async fn receive<T: From<Piece>>(
    stream: u32,
    mut writer: impl AsyncWrite + Unpin,
    mut inbound: mpsc::Receiver<Vec<u8>>,
    out: &mpsc::Sender<T>,
) {
    let mut written = 0;
    while let Some(data) = inbound.recv().await {
        if writer.write_all(&data).await.is_err() {
            return;
        }
        written += data.len() + FRAME_COST;
        if written >= GRANT {
            let bytes = u32::try_from(written).unwrap_or(u32::MAX);
            if out.send(Piece::Credit(stream, bytes).into()).await.is_err() {
                return;
            }
            written = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn credit(piece: &Piece) -> Option<u32> {
        match piece {
            Piece::Credit(1, bytes) => Some(*bytes),
            _ => None,
        }
    }

    #[tokio::test]
    async fn a_pump_sends_only_within_its_window_until_credit_arrives() {
        let mut lanes = Lanes::default();
        let (lane, pipe) = lane();
        lanes.insert(1, lane);
        let (local, mut app) = tokio::io::duplex(64 * 1024);
        let (out, mut sent) = mpsc::channel::<Piece>(1024);
        tokio::spawn(pump(1, local, pipe, out));
        tokio::spawn(async move { app.write_all(&vec![b'x'; 1024 * 1024]).await });
        let mut spent = 0;
        while let Ok(Some(Piece::Data(_, data))) =
            tokio::time::timeout(Duration::from_millis(200), sent.recv()).await
        {
            spent += data.len() + FRAME_COST;
        }
        assert!(
            spent > WINDOW - CHUNK - FRAME_COST && spent <= WINDOW,
            "{spent}"
        );
        lanes.receive(&Frame::Credit(1, u32::try_from(spent).unwrap()));
        let more = tokio::time::timeout(Duration::from_secs(5), sent.recv()).await;
        assert!(matches!(more, Ok(Some(Piece::Data(1, _)))));
    }

    #[tokio::test]
    async fn written_bytes_are_granted_back_in_batches() {
        let (mut lanes, (lane, pipe)) = (Lanes::default(), lane());
        lanes.insert(1, lane);
        let (local, mut app) = tokio::io::duplex(1024 * 1024);
        let (out, mut sent) = mpsc::channel::<Piece>(1024);
        tokio::spawn(pump(1, local, pipe, out));
        for _ in 0..8 {
            lanes.receive(&Frame::Data(1, &[b'y'; CHUNK]));
        }
        let mut granted = 0;
        while granted < 8 * (CHUNK + FRAME_COST) - GRANT {
            let piece = tokio::time::timeout(Duration::from_secs(5), sent.recv())
                .await
                .unwrap()
                .unwrap();
            let bytes = credit(&piece).expect("only credit flows back");
            assert!(bytes as usize >= GRANT);
            lanes.outgoing(&piece);
            granted += bytes as usize;
        }
        let mut written = vec![0; 8 * CHUNK];
        app.read_exact(&mut written).await.unwrap();
        assert!(written.iter().all(|byte| *byte == b'y'));
        assert_eq!(lanes.0[&1].owed, 8 * (CHUNK + FRAME_COST) - granted);
    }

    #[test]
    fn a_peer_past_its_window_or_overgranting_loses_the_stream() {
        let mut lanes = Lanes::default();
        let (first, _first_pipe) = lane();
        let (second, _second_pipe) = lane();
        lanes.insert(1, first);
        lanes.insert(2, second);
        for _ in 0..WINDOW / (CHUNK + FRAME_COST) {
            lanes.receive(&Frame::Data(1, &[0; CHUNK]));
        }
        assert!(lanes.0.contains_key(&1));
        lanes.receive(&Frame::Data(1, &[0; CHUNK]));
        assert!(!lanes.0.contains_key(&1));
        lanes.receive(&Frame::Credit(2, 1));
        assert!(!lanes.0.contains_key(&2));
        lanes.receive(&Frame::Data(9, b"unknown streams are ignored"));
        lanes.receive(&Frame::Close(9));
    }
}
