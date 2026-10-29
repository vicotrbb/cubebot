//! virtio-net (virtio 1.2 §5.1) over a tap device. No offloads are offered, so every buffer is a
//! 12-byte header followed by one Ethernet frame.
use super::{Device, Slot, queue::Queue};
use crate::kvm::memory::GuestMemory;
use std::{
    fs::File,
    io::{self, Read, Write},
    sync::{Arc, Condvar},
};

const MTU: u64 = 1 << 3;
const MAC: u64 = 1 << 5;
const HEADER: usize = 12;
const RECEIVE: usize = 0;
const TRANSMIT: usize = 1;
/// The largest frame a tap without offloads passes: a 64 KiB packet and its Ethernet header.
const FRAME_LIMIT: usize = 65535 + 14;

pub struct Net {
    tap: File,
    config: [u8; 12],
    /// Signalled when the driver posts receive buffers.
    receivable: Arc<Condvar>,
}

impl Net {
    pub fn new(tap: File, mac: [u8; 6], mtu: u16) -> Self {
        let mut config = [0; 12];
        config[..6].copy_from_slice(&mac);
        config[10..].copy_from_slice(&mtu.to_le_bytes());
        Self {
            tap,
            config,
            receivable: Arc::default(),
        }
    }

    pub fn receivable(&self) -> Arc<Condvar> {
        self.receivable.clone()
    }

    /// Delivers one frame; returns false when the driver has no receive buffer for it yet.
    fn receive(
        &self,
        queues: &mut [Queue],
        memory: &GuestMemory,
        frame: &[u8],
    ) -> io::Result<bool> {
        let queue = &mut queues[RECEIVE];
        let Some(chain) = queue.pop(memory)? else {
            return Ok(false);
        };
        let mut written = 0;
        // A buffer too small for the frame is returned empty, which the driver counts as an error.
        if chain.writable_len() >= (HEADER + frame.len()) as u64 {
            let mut header = [0; HEADER];
            header[10] = 1; // num_buffers
            chain.write_at(memory, 0, &header)?;
            chain.write_at(memory, HEADER as u64, frame)?;
            written = HEADER + frame.len();
        }
        queue.push(memory, chain.head, written as u32)?;
        Ok(true)
    }

    fn transmit(&mut self, queue: &mut Queue, memory: &GuestMemory) -> io::Result<bool> {
        let mut used = false;
        let mut frame = vec![0; FRAME_LIMIT];
        while let Some(chain) = queue.pop(memory)? {
            let len = chain.readable_len().saturating_sub(HEADER as u64);
            if let Some(frame) = frame.get_mut(..len as usize) {
                chain.read_at(memory, HEADER as u64, frame)?;
                // A full or down tap drops the frame, as a NIC would.
                let _ = self.tap.write(frame);
            }
            queue.push(memory, chain.head, 0)?;
            used = true;
        }
        Ok(used)
    }
}

impl Device for Net {
    const TYPE: u32 = 1;
    const QUEUES: usize = 2;

    fn features(&self) -> u64 {
        MAC | MTU
    }

    fn config(&self) -> &[u8] {
        &self.config
    }

    fn notify(
        &mut self,
        queue: usize,
        queues: &mut [Queue],
        memory: &GuestMemory,
    ) -> io::Result<bool> {
        match queue {
            RECEIVE => {
                self.receivable.notify_all();
                Ok(false)
            }
            TRANSMIT => self.transmit(&mut queues[TRANSMIT], memory),
            _ => Ok(false),
        }
    }
}

/// Moves frames from the tap to the guest until the tap fails, waiting for receive buffers
/// rather than dropping frames.
pub fn receive(
    mut tap: File,
    slot: &Slot<Net>,
    receivable: &Condvar,
    memory: &GuestMemory,
) -> io::Result<()> {
    let mut frame = vec![0; FRAME_LIMIT];
    loop {
        let len = tap.read(&mut frame)?;
        let mut transport = slot.lock();
        while !transport.run(|net, queues| net.receive(queues, memory, &frame[..len]))? {
            transport = receivable.wait(transport).expect("device lock");
        }
        drop(transport);
        slot.irq.raise()?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kvm::virtio::queue::tests::{Ring, readable, writable};

    const RING: Ring = Ring(0x1000);
    use std::os::{fd::OwnedFd, unix::net::UnixDatagram};

    /// A datagram pair keeps frame boundaries, as a tap does.
    fn net() -> (Net, UnixDatagram) {
        let (tap, peer) = UnixDatagram::pair().unwrap();
        let net = Net::new(File::from(OwnedFd::from(tap)), [2, 0, 0, 0, 0, 1], 1450);
        (net, peer)
    }

    #[test]
    fn transmits_frames_to_the_tap() {
        let memory = GuestMemory::map(1 << 16).unwrap();
        let (mut net, peer) = net();
        assert_eq!(net.config()[..6], [2, 0, 0, 0, 0, 1]);
        assert_eq!(net.config()[10..], 1450u16.to_le_bytes());
        memory.write(0x4000 + HEADER as u64, b"frame").unwrap();
        let mut queues = [Queue::default(), RING.queue(4)];
        RING.offer(&memory, 0, 0, &[readable(0x4000, HEADER as u32 + 5)]);
        assert!(net.notify(TRANSMIT, &mut queues, &memory).unwrap());
        let mut sent = [0; 16];
        assert_eq!(peer.recv(&mut sent).unwrap(), 5);
        assert_eq!((&sent[..5], RING.used(&memory, 0)), (&b"frame"[..], (0, 0)));
    }

    #[test]
    fn receives_frames_into_posted_buffers() {
        let memory = GuestMemory::map(1 << 16).unwrap();
        let (net, _peer) = net();
        let mut queues = [RING.queue(4), Queue::default()];
        assert!(!net.receive(&mut queues, &memory, b"reply").unwrap());
        RING.offer(&memory, 0, 0, &[writable(0x5000, 8)]);
        RING.offer(&memory, 1, 1, &[writable(0x6000, 64)]);
        assert!(net.receive(&mut queues, &memory, b"reply").unwrap());
        assert_eq!(RING.used(&memory, 0), (0, 0));
        assert!(net.receive(&mut queues, &memory, b"reply").unwrap());
        assert_eq!(RING.used(&memory, 1), (1, HEADER as u32 + 5));
        let mut received = [0; HEADER + 5];
        memory.read(0x6000, &mut received).unwrap();
        assert_eq!((received[10], &received[HEADER..]), (1, &b"reply"[..]));
    }
}
