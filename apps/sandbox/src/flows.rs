//! The guest's traffic as the VMM relays it (docs/adr/agents.md#a-watchdog-that-notifies): bytes
//! per remote address and port, TCP connections opened, the names asked of DNS and the public
//! addresses reached without DNS returning them. Only headers are read, never payloads beyond a
//! DNS question or answer. The guest cannot reach this table, so even root inside it cannot hide
//! traffic from it. The orchestrator reads and clears it with `cubebot-sandbox network`.
use cubebot_protocol::watch::{Endpoint, Flow, Name, Traffic, public};
use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
    net::Ipv4Addr,
    os::unix::{fs::PermissionsExt, net::UnixListener},
    path::Path,
    sync::{Arc, Mutex},
    time::Instant,
};

const FLOWS: usize = 2048;
const NAMES: usize = 1024;
const UNRESOLVED: usize = 256;
const RESOLVED: usize = 16384;
const TCP: u8 = 6;
const UDP: u8 = 17;
const DNS: u16 = 53;

/// One IPv4 packet's addresses, ports and size, as the headers give them.
#[derive(Debug, PartialEq)]
struct Packet<'a> {
    protocol: u8,
    source: Ipv4Addr,
    destination: Ipv4Addr,
    source_port: u16,
    destination_port: u16,
    /// A TCP SYN without ACK: a connection being opened.
    opening: bool,
    /// The UDP payload, for DNS.
    payload: &'a [u8],
    length: u64,
}

#[derive(Default)]
struct Counter {
    sent: u64,
    received: u64,
    opened: u32,
}

pub struct Flows {
    since: Instant,
    flows: HashMap<(u8, Ipv4Addr, u16), Counter>,
    names: HashMap<String, u32>,
    resolved: HashSet<Ipv4Addr>,
    unresolved: HashSet<(Ipv4Addr, u16)>,
    overflow: u64,
}

impl Default for Flows {
    fn default() -> Self {
        Self {
            since: Instant::now(),
            flows: HashMap::new(),
            names: HashMap::new(),
            resolved: HashSet::new(),
            unresolved: HashSet::new(),
            overflow: 0,
        }
    }
}

impl Flows {
    /// A frame the guest sent.
    pub fn outbound(&mut self, frame: &[u8]) {
        let Some(packet) = packet(frame) else {
            return;
        };
        let key = (packet.protocol, packet.destination, packet.destination_port);
        let Some(counter) = self.counter(key) else {
            return;
        };
        counter.sent += packet.length;
        counter.opened += u32::from(packet.opening);
        if packet.opening
            && public(packet.destination)
            && !self.resolved.contains(&packet.destination)
        {
            self.unresolve(packet.destination, packet.destination_port);
        }
        if packet.protocol == UDP
            && packet.destination_port == DNS
            && let Some(name) = question(packet.payload)
        {
            self.ask(name);
        }
    }

    /// A frame sent to the guest.
    pub fn inbound(&mut self, frame: &[u8]) {
        let Some(packet) = packet(frame) else {
            return;
        };
        if packet.protocol == UDP && packet.source_port == DNS {
            if self.resolved.len() >= RESOLVED {
                self.resolved.clear();
            }
            self.resolved.extend(answers(packet.payload));
        }
        let key = (packet.protocol, packet.source, packet.source_port);
        if let Some(counter) = self.counter(key) {
            counter.received += packet.length;
        }
    }

    /// The summary since the previous one, which starts the next.
    pub fn take(&mut self) -> Traffic {
        let flows = self
            .flows
            .drain()
            .map(|((protocol, address, port), counter)| Flow {
                address: address.to_string(),
                port,
                protocol: if protocol == TCP { "tcp" } else { "udp" }.to_owned(),
                sent: counter.sent,
                received: counter.received,
                opened: counter.opened,
            })
            .collect();
        let traffic = Traffic {
            seconds: self.since.elapsed().as_secs(),
            flows,
            names: self
                .names
                .drain()
                .map(|(name, count)| Name { name, count })
                .collect(),
            unresolved: self
                .unresolved
                .drain()
                .map(|(address, port)| Endpoint {
                    address: address.to_string(),
                    port,
                })
                .collect(),
            overflow: std::mem::take(&mut self.overflow),
        };
        self.since = Instant::now();
        traffic
    }

    fn counter(&mut self, key: (u8, Ipv4Addr, u16)) -> Option<&mut Counter> {
        if self.flows.len() >= FLOWS && !self.flows.contains_key(&key) {
            self.overflow += 1;
            return None;
        }
        Some(self.flows.entry(key).or_default())
    }

    fn ask(&mut self, name: String) {
        if self.names.len() >= NAMES && !self.names.contains_key(&name) {
            self.overflow += 1;
            return;
        }
        *self.names.entry(name).or_default() += 1;
    }

    fn unresolve(&mut self, address: Ipv4Addr, port: u16) {
        if self.unresolved.len() >= UNRESOLVED {
            self.overflow += 1;
            return;
        }
        self.unresolved.insert((address, port));
    }
}

/// Serves each connection one summary, read and cleared; the socket is this sandbox's own.
pub fn serve(path: &Path, flows: Arc<Mutex<Flows>>) -> io::Result<()> {
    let _ = std::fs::remove_file(path);
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    std::thread::spawn(move || {
        for mut client in listener.incoming().flatten() {
            let traffic = flows
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take();
            if let Ok(text) = serde_json::to_vec(&traffic) {
                let _ = client.write_all(&text);
            }
        }
    });
    Ok(())
}

fn packet(frame: &[u8]) -> Option<Packet<'_>> {
    if frame.get(12..14)? != [0x08, 0x00] {
        return None;
    }
    let ip = frame.get(14..)?;
    let header = usize::from(ip.first()? & 0x0f) * 4;
    let total = usize::from(u16::from_be_bytes([*ip.get(2)?, *ip.get(3)?]));
    let fragment = u16::from_be_bytes([*ip.get(6)?, *ip.get(7)?]) & 0x1fff;
    let protocol = *ip.get(9)?;
    let address = |at: usize| -> Option<Ipv4Addr> {
        Some(Ipv4Addr::from(
            <[u8; 4]>::try_from(ip.get(at..at + 4)?).ok()?,
        ))
    };
    let transport = ip.get(header..total.min(ip.len()))?;
    let port = |at: usize| {
        Some(u16::from_be_bytes([
            *transport.get(at)?,
            *transport.get(at + 1)?,
        ]))
    };
    // Later fragments carry no ports; they still count toward the flow's bytes below.
    let ports = (fragment == 0 && matches!(protocol, TCP | UDP))
        .then(|| Some((port(0)?, port(2)?)))
        .flatten();
    let (source_port, destination_port) = ports.unwrap_or_default();
    let flags = (protocol == TCP).then(|| transport.get(13)).flatten();
    Some(Packet {
        protocol,
        source: address(12)?,
        destination: address(16)?,
        source_port,
        destination_port,
        opening: flags.is_some_and(|flags| flags & 0x12 == 0x02),
        payload: if protocol == UDP {
            transport.get(8..).unwrap_or_default()
        } else {
            &[]
        },
        length: frame.len() as u64,
    })
    .filter(|packet| matches!(packet.protocol, TCP | UDP))
}

/// Where a DNS name starts its next label, and the name read so far, from `at`.
fn labels(message: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut name = String::new();
    for _ in 0..128 {
        let length = usize::from(*message.get(at)?);
        if length == 0 {
            return Some((name, at + 1));
        }
        // A compression pointer ends the name in two bytes; its target is not needed here.
        if length & 0xc0 == 0xc0 {
            return Some((name, at + 2));
        }
        let label = message.get(at + 1..at + 1 + length)?;
        if !name.is_empty() {
            name.push('.');
        }
        name.push_str(&String::from_utf8_lossy(label).to_lowercase());
        at += 1 + length;
    }
    None
}

/// The first question's name of a DNS query.
fn question(message: &[u8]) -> Option<String> {
    let questions = u16::from_be_bytes([*message.get(4)?, *message.get(5)?]);
    (questions > 0)
        .then(|| labels(message, 12).map(|(name, _)| name))
        .flatten()
        .filter(|name| !name.is_empty())
}

/// The IPv4 addresses in a DNS response's answers.
fn answers(message: &[u8]) -> Vec<Ipv4Addr> {
    let count = |at: usize| -> usize {
        message.get(at..at + 2).map_or(0, |bytes| {
            usize::from(u16::from_be_bytes([bytes[0], bytes[1]]))
        })
    };
    let (questions, records) = (count(4), count(6));
    let mut at = 12;
    for _ in 0..questions.min(8) {
        let Some((_, next)) = labels(message, at) else {
            return Vec::new();
        };
        at = next + 4;
    }
    let mut found = Vec::new();
    for _ in 0..records.min(64) {
        let Some((_, next)) = labels(message, at) else {
            break;
        };
        let (kind, length) = (count(next), count(next + 8));
        let data = next + 10;
        if kind == 1
            && length == 4
            && let Some(bytes) = message.get(data..data + 4)
        {
            found.push(Ipv4Addr::new(bytes[0], bytes[1], bytes[2], bytes[3]));
        }
        at = data + length;
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUEST: [u8; 4] = [10, 0, 0, 2];

    fn frame(protocol: u8, from: [u8; 4], to: [u8; 4], ports: (u16, u16), rest: &[u8]) -> Vec<u8> {
        let mut transport = Vec::new();
        transport.extend(ports.0.to_be_bytes());
        transport.extend(ports.1.to_be_bytes());
        transport.extend(rest);
        let total = 20 + transport.len();
        let mut frame = vec![0; 12];
        frame.extend([0x08, 0x00, 0x45, 0, 0, 0]);
        frame[16..18].copy_from_slice(&(total as u16).to_be_bytes());
        frame.extend([0, 0, 0, 0, 64, protocol, 0, 0]);
        frame.extend(from);
        frame.extend(to);
        frame.extend(transport);
        frame
    }

    fn syn(to: [u8; 4], port: u16) -> Vec<u8> {
        // Sequence, acknowledgement, offset and the SYN flag.
        let mut rest = vec![0; 8];
        rest.extend([0x50, 0x02]);
        frame(TCP, GUEST, to, (40000, port), &rest)
    }

    fn dns(id: u8, answer: Option<[u8; 4]>) -> Vec<u8> {
        let mut message = vec![
            0,
            id,
            0x81,
            0x80,
            0,
            1,
            0,
            u8::from(answer.is_some()),
            0,
            0,
            0,
            0,
        ];
        message.extend(b"\x07example\x03com\x00\x00\x01\x00\x01");
        if let Some(address) = answer {
            message.extend([0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4]);
            message.extend(address);
        }
        let mut udp = vec![0, 0, 0, 0];
        udp.extend(message);
        udp
    }

    #[test]
    fn traffic_is_counted_per_remote_end_and_cleared_on_read() {
        let mut flows = Flows::default();
        flows.outbound(&syn([93, 184, 216, 34], 443));
        let reply = frame(TCP, [93, 184, 216, 34], GUEST, (443, 40000), &[0; 16]);
        flows.inbound(&reply);
        flows.outbound(&frame(
            UDP,
            GUEST,
            [10, 43, 0, 10],
            (5353, 53),
            &dns(1, None),
        ));
        let traffic = flows.take();
        let https = traffic.flows.iter().find(|flow| flow.port == 443).unwrap();
        assert_eq!((https.opened, https.received), (1, reply.len() as u64));
        assert_eq!(https.protocol, "tcp");
        assert_eq!(
            traffic.names,
            [Name {
                name: "example.com".into(),
                count: 1
            }]
        );
        assert_eq!(
            traffic.unresolved,
            [Endpoint {
                address: "93.184.216.34".into(),
                port: 443
            }],
            "no DNS answer named it"
        );
        assert!(flows.take().flows.is_empty());
    }

    #[test]
    fn an_address_dns_returned_is_resolved() {
        let mut flows = Flows::default();
        let answer = dns(2, Some([93, 184, 216, 34]));
        flows.inbound(&frame(UDP, [10, 43, 0, 10], GUEST, (53, 5353), &answer));
        flows.outbound(&syn([93, 184, 216, 34], 443));
        flows.outbound(&syn([10, 0, 0, 9], 22));
        assert!(
            flows.take().unresolved.is_empty(),
            "private addresses are not counted"
        );
    }

    #[test]
    fn malformed_frames_are_ignored_and_tables_are_bounded() {
        let mut flows = Flows::default();
        let whole = syn([1, 1, 1, 1], 443);
        for length in 0..whole.len() {
            flows.outbound(&whole[..length]);
            flows.inbound(&whole[..length]);
        }
        let mut lying = syn([1, 1, 1, 1], 443);
        lying[14] = 0x4f; // a header longer than the packet
        flows.outbound(&lying);
        for port in 0..(FLOWS as u16 + 10) {
            flows.outbound(&syn([8, 8, 8, 8], port));
        }
        let traffic = flows.take();
        assert_eq!(traffic.flows.len(), FLOWS);
        assert!(traffic.overflow >= 10);
        assert_eq!(
            answers(b"\0\0\0\0\0\x01\0\x01\0\0\0\0\xff"),
            Vec::<Ipv4Addr>::new()
        );
    }
}
