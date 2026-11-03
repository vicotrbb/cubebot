//! The host channel (ADR 0085): each connection on the Unix socket that `cubebot-sandbox
//! desktop` relays for the orchestrator is paired with one of the streams the guest's arbiter
//! keeps open to the desktop host port, and bytes pass both ways unparsed. The arbiter treats
//! each stream as one host channel connection, so its control and frame state end with it.
use super::virtio::{Slot, vsock::Vsock};
use crate::kvm::memory::GuestMemory;
use std::{
    io::{self, Read, Write},
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::Path,
    sync::{Arc, mpsc::Receiver},
    time::{Duration, Instant},
};

/// How long a connection waits for the guest's arbiter, such as while the guest boots.
const WAIT: Duration = Duration::from_secs(30);

/// Accepts host channel connections on `path` and pairs them in order with the streams in
/// `spares`, on threads of their own.
pub fn listen(
    path: &Path,
    vsock: Arc<Slot<Vsock>>,
    spares: Receiver<u32>,
    memory: &'static GuestMemory,
) -> io::Result<()> {
    // A socket left by an earlier run of this sandbox; the path is this sandbox's own.
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    std::thread::spawn(move || {
        for client in listener.incoming().flatten() {
            let Some((port, bytes)) = pair(&vsock, &spares) else {
                eprintln!("Desktop host channel refused: no guest desktop within {WAIT:?}");
                continue;
            };
            if let Err(error) = bridge(client, port, bytes, vsock.clone(), memory) {
                eprintln!("Desktop host channel closed: {error}");
            }
        }
    });
    Ok(())
}

/// Copies the guest stream's bytes to the connection and the connection's to the stream, each
/// on its own thread.
fn bridge(
    client: UnixStream,
    port: u32,
    bytes: Receiver<Vec<u8>>,
    vsock: Arc<Slot<Vsock>>,
    memory: &'static GuestMemory,
) -> io::Result<()> {
    let to_guest = client.try_clone()?;
    let from_guest = vsock.clone();
    std::thread::spawn(move || {
        for chunk in bytes {
            if (&client).write_all(&chunk).is_err() {
                break;
            }
            deliver(
                &from_guest,
                |vsock| vsock.consumed(port, chunk.len()),
                memory,
            );
        }
        // The stream closed, or the connection did; either ends both.
        let _ = client.shutdown(std::net::Shutdown::Both);
    });
    std::thread::spawn(move || forward(to_guest, port, &vsock, memory));
    Ok(())
}

/// The next guest stream that is still open, waiting at most [`WAIT`] in all.
fn pair(vsock: &Slot<Vsock>, spares: &Receiver<u32>) -> Option<(u32, Receiver<Vec<u8>>)> {
    let deadline = Instant::now() + WAIT;
    loop {
        let port = spares
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .ok()?;
        let (bridge, bytes) = std::sync::mpsc::channel();
        if vsock.lock().device.pair(port, bridge) {
            return Some((port, bytes));
        }
    }
}

/// Copies the connection's bytes to the guest stream until it ends, then shuts the stream's
/// host side so the arbiter sees the end after answering what it has.
/// shortcut: the stream's outbox grows with whatever the orchestrator sends unanswered; bound it
/// if a host channel client other than the orchestrator appears.
fn forward(mut client: UnixStream, port: u32, vsock: &Slot<Vsock>, memory: &GuestMemory) {
    let mut buffer = vec![0; 64 << 10];
    loop {
        match client.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(read) => deliver(vsock, |vsock| vsock.send(port, &buffer[..read]), memory),
        }
    }
    deliver(vsock, |vsock| vsock.end(port), memory);
}

fn deliver(vsock: &Slot<Vsock>, work: impl FnOnce(&mut Vsock), memory: &GuestMemory) {
    let delivered = vsock.run(|vsock, queues| {
        work(vsock);
        vsock.deliver(queues, memory)
    });
    if let Err(error) = delivered {
        super::fail(&format!("desktop host channel: {error}"));
    }
}
