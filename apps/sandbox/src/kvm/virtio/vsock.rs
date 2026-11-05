//! virtio-vsock (virtio 1.2 §5.10): one control stream and a few host channel streams from reserved ports; every other packet is reset.
use super::{Device, queue::Queue};
use crate::kvm::memory::{GuestMemory, invalid};
use cubebot_protocol::{
    desktop,
    guest::{GUEST_CID, HOST_CID, LINE_LIMIT, PORT, Report},
};
use std::{
    collections::VecDeque,
    io,
    sync::mpsc::{Sender, SyncSender},
};

const HEADER: usize = 44;
const STREAM: u16 = 1;
const REQUEST: u16 = 1;
const RESPONSE: u16 = 2;
const RESET: u16 = 3;
const SHUTDOWN: u16 = 4;
const DATA: u16 = 5;
const CREDIT_UPDATE: u16 = 6;
const CREDIT_REQUEST: u16 = 7;
/// The host sends no more, so the guest sees end of stream but can still report its exit.
const SHUTDOWN_SEND: u32 = 2;
const BUFFER: u32 = 64 << 10;
const RECEIVE: usize = 0;
const TRANSMIT: usize = 1;
pub const DESKTOP_STREAMS: usize = 16;
/// Transmit waits while this many replies wait for receive buffers, so a guest that stops taking them cannot grow the host's memory.
const REPLIES: usize = 1024;
/// The guest kernel's last reserved vsock port; binding one takes root.
const RESERVED: u32 = 1023;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Header {
    src_cid: u64,
    dst_cid: u64,
    src_port: u32,
    dst_port: u32,
    len: u32,
    kind: u16,
    op: u16,
    flags: u32,
    buf_alloc: u32,
    fwd_cnt: u32,
}

impl Header {
    fn parse(raw: &[u8; HEADER]) -> Self {
        let u16_at = |at: usize| u16::from_le_bytes([raw[at], raw[at + 1]]);
        let u32_at = |at: usize| u32::from(u16_at(at)) | u32::from(u16_at(at + 2)) << 16;
        let u64_at = |at: usize| u64::from(u32_at(at)) | u64::from(u32_at(at + 4)) << 32;
        Self {
            src_cid: u64_at(0),
            dst_cid: u64_at(8),
            src_port: u32_at(16),
            dst_port: u32_at(20),
            len: u32_at(24),
            kind: u16_at(28),
            op: u16_at(30),
            flags: u32_at(32),
            buf_alloc: u32_at(36),
            fwd_cnt: u32_at(40),
        }
    }

    fn encode(&self) -> [u8; HEADER] {
        let mut raw = [0; HEADER];
        raw[..8].copy_from_slice(&self.src_cid.to_le_bytes());
        raw[8..16].copy_from_slice(&self.dst_cid.to_le_bytes());
        raw[16..20].copy_from_slice(&self.src_port.to_le_bytes());
        raw[20..24].copy_from_slice(&self.dst_port.to_le_bytes());
        raw[24..28].copy_from_slice(&self.len.to_le_bytes());
        raw[28..30].copy_from_slice(&self.kind.to_le_bytes());
        raw[30..32].copy_from_slice(&self.op.to_le_bytes());
        raw[32..36].copy_from_slice(&self.flags.to_le_bytes());
        raw[36..40].copy_from_slice(&self.buf_alloc.to_le_bytes());
        raw[40..].copy_from_slice(&self.fwd_cnt.to_le_bytes());
        raw
    }

    fn reply(&self, op: u16) -> Self {
        Self {
            src_cid: HOST_CID.into(),
            dst_cid: GUEST_CID.into(),
            src_port: self.dst_port,
            dst_port: self.src_port,
            kind: STREAM,
            op,
            ..Self::default()
        }
    }
}

/// One open stream and both directions of its credit (§5.10.6.3).
struct Connection {
    port: u32,
    service: u32,
    peer_buffer: u32,
    peer_forwarded: u32,
    sent: u32,
    forwarded: u32,
    outbox: VecDeque<u8>,
    line: Vec<u8>,
    bridge: Option<Sender<Vec<u8>>>,
    received: u32,
    ending: bool,
}

impl Connection {
    fn sendable(&self) -> usize {
        let credit = self
            .peer_buffer
            .wrapping_sub(self.sent.wrapping_sub(self.peer_forwarded));
        self.outbox.len().min(credit as usize)
    }

    fn lines(&mut self, payload: &[u8], limit: usize) -> Option<Vec<Vec<u8>>> {
        self.forwarded = self.forwarded.wrapping_add(payload.len() as u32);
        self.line.extend_from_slice(payload);
        let mut lines = Vec::new();
        while let Some(end) = self.line.iter().position(|&byte| byte == b'\n') {
            lines.push(self.line.drain(..=end).take(end).collect());
        }
        (self.line.len() <= limit).then_some(lines)
    }

    fn shutdown(&self) -> Header {
        Header {
            flags: SHUTDOWN_SEND,
            ..self.header(SHUTDOWN)
        }
    }

    fn header(&self, op: u16) -> Header {
        let request = Header {
            src_port: self.port,
            dst_port: self.service,
            ..Header::default()
        };
        request.reply(op)
    }
}

pub struct Vsock {
    config: [u8; 8],
    environment: Vec<u8>,
    connections: Vec<Connection>,
    spares: SyncSender<u32>,
    replies: VecDeque<Header>,
    alive: bool,
    exit: Option<u8>,
}

impl Vsock {
    pub fn new(environment: Vec<u8>, spares: SyncSender<u32>) -> Self {
        Self {
            config: u64::from(GUEST_CID).to_le_bytes(),
            environment,
            connections: Vec::new(),
            spares,
            replies: VecDeque::new(),
            alive: false,
            exit: None,
        }
    }

    pub fn exit(&self) -> Option<u8> {
        self.exit
    }

    fn control(&mut self) -> Option<&mut Connection> {
        self.connections.iter_mut().find(|c| c.service == PORT)
    }

    pub fn refresh(&mut self, environment: Vec<u8>, changed: &[u8]) {
        self.environment = environment;
        if let Some(connection) = self.control() {
            connection.outbox.extend(changed);
        }
    }

    fn host_channel(&mut self, port: u32) -> Option<&mut Connection> {
        self.connections
            .iter_mut()
            .find(|c| c.service == desktop::HOST_PORT && c.port == port)
    }

    pub fn pair(&mut self, port: u32, bridge: Sender<Vec<u8>>) -> bool {
        match self.host_channel(port) {
            Some(connection) if connection.bridge.is_none() => {
                connection.bridge = Some(bridge);
                true
            }
            _ => false,
        }
    }

    pub fn send(&mut self, port: u32, bytes: &[u8]) {
        if let Some(connection) = self.host_channel(port) {
            connection.outbox.extend(bytes);
        }
    }

    pub fn consumed(&mut self, port: u32, bytes: usize) {
        let Some(connection) = self.host_channel(port) else {
            return;
        };
        connection.forwarded = connection.forwarded.wrapping_add(bytes as u32);
        let update = connection.header(CREDIT_UPDATE);
        self.replies.push_back(update);
    }

    pub fn end(&mut self, port: u32) {
        let Some(connection) = self.host_channel(port) else {
            return;
        };
        connection.ending = true;
        if connection.outbox.is_empty() {
            let shutdown = connection.shutdown();
            self.replies.push_back(shutdown);
        }
    }

    pub fn shutdown(&mut self) -> bool {
        let Some(connection) = self.control() else {
            return false;
        };
        let shutdown = connection.shutdown();
        self.replies.push_back(shutdown);
        true
    }

    fn handle(&mut self, header: Header, payload: &[u8]) {
        match (header.op, self.current(&header)) {
            (RESET, Some(index)) => self.close(index),
            (RESET, None) => (),
            (REQUEST, None) if self.admits(&header) => self.open(header),
            (_, Some(index)) => self.serve(index, header, payload),
            _ => self.replies.push_back(header.reply(RESET)),
        }
    }

    fn current(&self, header: &Header) -> Option<usize> {
        if !addressed(header) {
            return None;
        }
        self.connections
            .iter()
            .position(|c| c.port == header.src_port && c.service == header.dst_port)
    }

    /// Only reserved source ports, so only root in the guest, which is the arbiter, opens host channel streams.
    fn admits(&self, header: &Header) -> bool {
        let open = |service| {
            self.connections
                .iter()
                .filter(|c| c.service == service)
                .count()
        };
        addressed(header)
            && match header.dst_port {
                PORT => open(PORT) == 0,
                desktop::HOST_PORT => {
                    header.src_port <= RESERVED && open(desktop::HOST_PORT) < DESKTOP_STREAMS
                }
                _ => false,
            }
    }

    fn refuse(&mut self, header: Header) {
        if let Some(index) = self.current(&header) {
            self.close(index);
        }
        self.replies.push_back(header.reply(RESET));
    }

    #[tracing::instrument(
        name = "vsock_connection",
        skip_all,
        fields(port = request.src_port, service = request.dst_port)
    )]
    fn open(&mut self, request: Header) {
        let outbox = match request.dst_port {
            PORT => self.environment.iter().copied().collect(),
            _ => VecDeque::new(),
        };
        // A full queue means no client has paired streams for a while; the arbiter dials again.
        if request.dst_port == desktop::HOST_PORT && self.spares.try_send(request.src_port).is_err()
        {
            return self.replies.push_back(request.reply(RESET));
        }
        self.connections.push(Connection {
            port: request.src_port,
            service: request.dst_port,
            peer_buffer: request.buf_alloc,
            peer_forwarded: request.fwd_cnt,
            sent: 0,
            forwarded: 0,
            outbox,
            line: Vec::new(),
            bridge: None,
            received: 0,
            ending: false,
        });
        self.replies.push_back(request.reply(RESPONSE));
    }

    fn serve(&mut self, index: usize, header: Header, payload: &[u8]) {
        let connection = &mut self.connections[index];
        connection.peer_buffer = header.buf_alloc;
        connection.peer_forwarded = header.fwd_cnt;
        match header.op {
            DATA if connection.service == PORT => match connection.lines(payload, LINE_LIMIT) {
                Some(lines) => {
                    for line in lines {
                        match serde_json::from_slice(&line) {
                            Ok(report) => self.record(&report),
                            Err(_) => return self.refuse(header),
                        }
                    }
                }
                None => return self.refuse(header),
            },
            DATA => return self.bridged(index, header, payload),
            CREDIT_REQUEST => (),
            CREDIT_UPDATE => return,
            _ => return self.refuse(header),
        }
        self.replies.push_back(header.reply(CREDIT_UPDATE));
    }

    /// A stream that speaks before it is paired, outruns its credit or whose connection is gone is reset.
    fn bridged(&mut self, index: usize, header: Header, payload: &[u8]) {
        let connection = &mut self.connections[index];
        connection.received = connection.received.wrapping_add(payload.len() as u32);
        let within = connection.received.wrapping_sub(connection.forwarded) <= BUFFER;
        let passed = connection
            .bridge
            .as_ref()
            .is_some_and(|bridge| bridge.send(payload.to_vec()).is_ok());
        if !(within && passed) {
            self.refuse(header);
        }
    }

    fn record(&mut self, report: &Report) {
        match report {
            Report::Liveness(_) if !std::mem::replace(&mut self.alive, true) => {
                eprintln!("Guest agent alive");
            }
            Report::Liveness(_) => (),
            Report::Exit { exit } => {
                eprintln!("Guest agent exited with status {exit}");
                self.exit = Some(*exit);
            }
        }
    }

    fn close(&mut self, index: usize) {
        let closed = self.connections.swap_remove(index);
        if closed.service == PORT && std::mem::take(&mut self.alive) {
            eprintln!("Guest control channel closed");
        }
    }

    fn transmit(&mut self, queue: &mut Queue, memory: &GuestMemory) -> io::Result<bool> {
        let mut used = false;
        while self.replies.len() < REPLIES
            && let Some(chain) = queue.pop(memory)?
        {
            let mut raw = [0; HEADER];
            if chain.read_at(memory, 0, &mut raw)? < HEADER {
                return Err(invalid("malformed vsock packet"));
            }
            let header = Header::parse(&raw);
            // Within the advertised buffer, so a guest cannot make the host allocate more.
            let mut payload = vec![0; header.len.min(BUFFER) as usize];
            let read = chain.read_at(memory, HEADER as u64, &mut payload)?;
            if read == header.len as usize {
                self.handle(header, &payload);
            } else {
                self.refuse(header);
            }
            queue.push(memory, chain.head, 0)?;
            used = true;
        }
        Ok(used)
    }

    pub fn deliver(&mut self, queues: &mut [Queue], memory: &GuestMemory) -> io::Result<bool> {
        let queue = &mut queues[RECEIVE];
        let mut used = false;
        while self.pending() && queue.has_available(memory)? {
            let chain = queue.pop(memory)?;
            let chain = chain.ok_or_else(|| invalid("vsock receive queue has no buffer"))?;
            let room = chain.writable_len().checked_sub(HEADER as u64);
            let room = room.ok_or_else(|| invalid("vsock receive buffer too small"))?;
            let (mut header, payload) = match self.replies.pop_front() {
                Some(header) => (header, Vec::new()),
                None => self
                    .data(room as usize)
                    .ok_or_else(|| invalid("vsock has no pending data"))?,
            };
            let connection = self
                .connections
                .iter()
                .find(|c| c.port == header.dst_port && c.service == header.src_port);
            header.buf_alloc = BUFFER;
            header.fwd_cnt = connection.map_or(0, |connection| connection.forwarded);
            header.len = payload.len() as u32;
            chain.write_at(memory, 0, &header.encode())?;
            chain.write_at(memory, HEADER as u64, &payload)?;
            queue.push(memory, chain.head, (HEADER + payload.len()) as u32)?;
            used = true;
        }
        Ok(used)
    }

    fn pending(&self) -> bool {
        !self.replies.is_empty() || self.connections.iter().any(|c| c.sendable() > 0)
    }

    fn data(&mut self, room: usize) -> Option<(Header, Vec<u8>)> {
        let connection = self.connections.iter_mut().find(|c| c.sendable() > 0)?;
        let len = connection.sendable().min(room);
        let payload: Vec<u8> = connection.outbox.drain(..len).collect();
        connection.sent = connection.sent.wrapping_add(len as u32);
        let header = connection.header(DATA);
        if connection.ending && connection.outbox.is_empty() {
            let shutdown = connection.shutdown();
            self.replies.push_back(shutdown);
        }
        Some((header, payload))
    }
}

fn addressed(header: &Header) -> bool {
    header.kind == STREAM
        && header.src_cid == u64::from(GUEST_CID)
        && header.dst_cid == u64::from(HOST_CID)
}

impl Device for Vsock {
    const TYPE: u32 = 19;
    const QUEUES: usize = 3;

    fn features(&self) -> u64 {
        0
    }

    fn config(&self) -> &[u8] {
        &self.config
    }

    fn notify(
        &mut self,
        _queue: usize,
        queues: &mut [Queue],
        memory: &GuestMemory,
    ) -> io::Result<bool> {
        let flushed = self.deliver(queues, memory)?;
        // Transmit waits while replies do, so any notification, such as new receive buffers, resumes it.
        let transmitted = queues[TRANSMIT].ready && self.transmit(&mut queues[TRANSMIT], memory)?;
        // The event queue only ever holds buffers; this device raises no events.
        Ok(self.deliver(queues, memory)? | flushed | transmitted)
    }

    fn reset(&mut self) {
        while !self.connections.is_empty() {
            self.close(0);
        }
        self.replies.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kvm::virtio::queue::tests::{Ring, readable, writable};

    const RX: Ring = Ring(0x1000);
    const TX: Ring = Ring(0x2000);

    struct Guest {
        memory: GuestMemory,
        queues: [Queue; 3],
        vsock: Vsock,
        spares: std::sync::mpsc::Receiver<u32>,
        sent: u16,
        posted: u16,
        received: u16,
    }

    impl Guest {
        fn new() -> Self {
            let (sender, spares) = std::sync::mpsc::sync_channel(DESKTOP_STREAMS);
            Self {
                memory: GuestMemory::map(1 << 20).unwrap(),
                queues: [RX.queue(64), TX.queue(64), Queue::default()],
                vsock: Vsock::new(b"A=1\0\0".to_vec(), sender),
                spares,
                sent: 0,
                posted: 0,
                received: 0,
            }
        }

        fn post(&mut self, count: u16) {
            for _ in 0..count {
                let buffer = writable(0x10000 + 0x1000 * u64::from(self.posted), 4096);
                RX.offer(&self.memory, self.posted, self.posted, &[buffer]);
                self.posted += 1;
            }
        }

        fn send(&mut self, op: u16, port: u32, credit: (u32, u32), payload: &[u8]) {
            self.send_from(5000, op, port, credit, payload);
        }

        fn send_from(&mut self, from: u32, op: u16, port: u32, credit: (u32, u32), payload: &[u8]) {
            self.offer(from, op, port, credit, payload);
            assert!(
                self.vsock
                    .notify(TRANSMIT, &mut self.queues, &self.memory)
                    .unwrap()
            );
        }

        fn offer(&mut self, from: u32, op: u16, port: u32, credit: (u32, u32), payload: &[u8]) {
            let header = Header {
                src_cid: GUEST_CID.into(),
                dst_cid: HOST_CID.into(),
                src_port: from,
                dst_port: port,
                len: payload.len() as u32,
                kind: STREAM,
                op,
                buf_alloc: credit.0,
                fwd_cnt: credit.1,
                ..Header::default()
            };
            let address = 0x8000 + 0x100 * u64::from(self.sent);
            self.memory.write(address, &header.encode()).unwrap();
            self.memory.write(address + HEADER as u64, payload).unwrap();
            let packet = readable(address, (HEADER + payload.len()) as u32);
            TX.offer(&self.memory, self.sent, self.sent, &[packet]);
            self.sent += 1;
        }

        fn receive(&mut self) -> Vec<(Header, Vec<u8>)> {
            let used = self.memory.load_u16(0x1c02).unwrap();
            let packets = (self.received..used).map(|slot| {
                let (head, written) = RX.used(&self.memory, slot);
                let address = 0x10000 + 0x1000 * u64::from(head);
                let mut raw = [0; HEADER];
                self.memory.read(address, &mut raw).unwrap();
                let mut payload = vec![0; written as usize - HEADER];
                self.memory
                    .read(address + HEADER as u64, &mut payload)
                    .unwrap();
                (Header::parse(&raw), payload)
            });
            let packets = packets.collect();
            self.received = used;
            packets
        }
    }

    fn ops(packets: &[(Header, Vec<u8>)]) -> Vec<(u16, &[u8])> {
        packets
            .iter()
            .map(|(header, payload)| (header.op, payload.as_slice()))
            .collect()
    }

    #[test]
    fn serves_the_environment_within_credit_and_reads_liveness() {
        let mut guest = Guest::new();
        guest.post(3);
        guest.send(REQUEST, PORT, (4, 0), &[]);
        let packets = guest.receive();
        assert_eq!(ops(&packets), [(RESPONSE, &b""[..]), (DATA, b"A=1\0")]);
        assert_eq!((packets[1].0.src_port, packets[1].0.dst_port), (PORT, 5000));
        guest.send(CREDIT_UPDATE, PORT, (4, 4), &[]);
        assert_eq!(ops(&guest.receive()), [(DATA, &b"\0"[..])]);
        guest.post(2);
        let line = b"{\"status\":\"alive\"}\n";
        guest.send(DATA, PORT, (4, 5), line);
        let packets = guest.receive();
        assert_eq!(ops(&packets), [(CREDIT_UPDATE, &b""[..])]);
        assert_eq!(packets[0].0.fwd_cnt, line.len() as u32);
        assert!(guest.vsock.alive);
        assert!(guest.vsock.shutdown());
        guest
            .vsock
            .deliver(&mut guest.queues, &guest.memory)
            .unwrap();
        let packets = guest.receive();
        assert_eq!(
            (packets[0].0.op, packets[0].0.flags),
            (SHUTDOWN, SHUTDOWN_SEND)
        );
        guest.send(DATA, PORT, (4, 5), b"{\"exit\":143}\n");
        assert_eq!(guest.vsock.exit(), Some(143));
        guest.send(RESET, PORT, (4, 5), &[]);
        assert!(guest.vsock.connections.is_empty() && !guest.vsock.alive);
        assert!(!guest.vsock.shutdown());
    }

    #[test]
    fn refreshes_files_on_the_open_stream() {
        let mut guest = Guest::new();
        guest.post(4);
        guest.vsock.refresh(b"A=2\0\0".to_vec(), b"unused");
        guest.send(REQUEST, PORT, (4096, 0), &[]);
        assert_eq!(ops(&guest.receive())[1], (DATA, &b"A=2\0\0"[..]));
        guest.vsock.refresh(b"A=3\0\0".to_vec(), b"A=3\0\0");
        guest
            .vsock
            .deliver(&mut guest.queues, &guest.memory)
            .unwrap();
        assert_eq!(ops(&guest.receive()), [(DATA, &b"A=3\0\0"[..])]);
    }

    #[test]
    fn resets_other_ports_and_broken_streams() {
        let mut guest = Guest::new();
        guest.post(8);
        guest.send(REQUEST, 9, (4096, 0), &[]);
        assert_eq!(ops(&guest.receive()), [(RESET, &b""[..])]);
        guest.send(REQUEST, PORT, (4096, 0), &[]);
        guest.receive();
        guest.send(DATA, PORT, (4096, 0), b"not liveness\n");
        assert_eq!(ops(&guest.receive()), [(RESET, &b""[..])]);
        assert!(guest.vsock.connections.is_empty());
        guest.send(REQUEST, PORT, (4096, 0), &[]);
        guest.receive();
        guest.send(DATA, PORT, (4096, 0), &[b'x'; LINE_LIMIT + 1]);
        assert_eq!(ops(&guest.receive()), [(RESET, &b""[..])]);
    }

    #[test]
    fn bridges_host_channel_streams_from_reserved_ports() {
        let mut guest = Guest::new();
        guest.post(16);
        let port = desktop::HOST_PORT;
        guest.send_from(5000, REQUEST, port, (4096, 0), &[]);
        guest.send_from(900, REQUEST, port, (4096, 0), &[]);
        let replies: Vec<_> = ops(&guest.receive())
            .into_iter()
            .map(|(op, _)| op)
            .collect();
        assert_eq!(
            replies,
            [RESET, RESPONSE],
            "only a reserved port is admitted"
        );
        assert_eq!(guest.spares.try_iter().collect::<Vec<_>>(), [900]);
        let (bridge, bytes) = std::sync::mpsc::channel();
        assert!(guest.vsock.pair(900, bridge.clone()));
        assert!(!guest.vsock.pair(900, bridge), "a stream pairs once");
        guest.send_from(900, DATA, port, (4096, 0), b"{\"mode\":");
        assert_eq!(bytes.try_recv().unwrap(), b"{\"mode\":");
        assert!(
            guest.receive().is_empty(),
            "no credit before the connection takes it"
        );
        guest.vsock.consumed(900, 8);
        guest.vsock.send(900, b"frame\n");
        guest.vsock.end(900);
        guest
            .vsock
            .deliver(&mut guest.queues, &guest.memory)
            .unwrap();
        let packets = guest.receive();
        assert_eq!(
            ops(&packets),
            [
                (CREDIT_UPDATE, &b""[..]),
                (DATA, b"frame\n"),
                (SHUTDOWN, b"")
            ]
        );
        assert_eq!(packets[0].0.fwd_cnt, 8);
        assert_eq!(packets[2].0.flags, SHUTDOWN_SEND);
        guest.send_from(901, REQUEST, port, (4096, 0), &[]);
        guest.receive();
        guest.send_from(901, DATA, port, (4096, 0), b"unpaired");
        assert_eq!(ops(&guest.receive()), [(RESET, &b""[..])]);
        drop(bytes);
        guest.send_from(900, DATA, port, (4096, 8), b"gone");
        assert_eq!(ops(&guest.receive()), [(RESET, &b""[..])]);
        assert!(guest.vsock.connections.is_empty());
    }

    #[test]
    fn transmit_waits_while_replies_wait_for_buffers() {
        let mut guest = Guest::new();
        guest.vsock.replies.extend([Header::default(); REPLIES]);
        guest.offer(5000, REQUEST, 9, (4096, 0), &[]);
        let notify = |guest: &mut Guest, queue| {
            let Guest {
                vsock,
                queues,
                memory,
                ..
            } = guest;
            vsock.notify(queue, queues, memory).unwrap()
        };
        assert!(!notify(&mut guest, TRANSMIT), "the request waits");
        assert_eq!(guest.vsock.replies.len(), REPLIES);
        guest.post(64);
        assert!(notify(&mut guest, RECEIVE));
        assert_eq!(guest.vsock.replies.len(), REPLIES - 64 + 1);
        assert_eq!(guest.vsock.replies.back().unwrap().op, RESET);
    }

    #[test]
    fn refuses_host_channel_streams_once_unpaired_ones_queue_up() {
        let mut guest = Guest::new();
        let port = desktop::HOST_PORT;
        for from in 900..900 + DESKTOP_STREAMS as u32 {
            guest.send_from(from, REQUEST, port, (4096, 0), &[]);
            guest.send_from(from, RESET, port, (4096, 0), &[]);
        }
        guest.send_from(950, REQUEST, port, (4096, 0), &[]);
        assert!(guest.vsock.connections.is_empty());
        assert_eq!(guest.vsock.replies.back().unwrap().op, RESET);
        assert_eq!(guest.spares.try_iter().count(), DESKTOP_STREAMS);
    }
}
