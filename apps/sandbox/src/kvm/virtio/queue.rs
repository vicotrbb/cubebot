//! Split virtqueues (virtio 1.2 §2.7), shared by every device. Indirect descriptors and event
//! suppression are not offered, so each chain is a plain list in the descriptor table.
use super::super::memory::{GuestMemory, invalid};
use std::io;

pub const MAX_SIZE: u16 = 256;
const NEXT: u16 = 1;
const WRITE: u16 = 2;

#[derive(Default)]
pub struct Queue {
    pub size: u16,
    pub ready: bool,
    pub descriptors: u64,
    pub driver: u64,
    pub device: u64,
    next_available: u16,
    next_used: u16,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Descriptor {
    pub address: u64,
    pub len: u32,
    pub writable: bool,
}

/// One request: the device reads its readable descriptors and writes its writable ones, each as a
/// contiguous byte stream.
pub struct Chain {
    pub head: u16,
    pub descriptors: Vec<Descriptor>,
}

impl Queue {
    /// Whether the driver may enable the queue: a power-of-two size and the alignment §2.7 requires.
    pub fn valid(&self) -> bool {
        self.size.is_power_of_two()
            && self.size <= MAX_SIZE
            && self.descriptors.is_multiple_of(16)
            && self.driver.is_multiple_of(2)
            && self.device.is_multiple_of(4)
    }

    pub fn has_available(&self, memory: &GuestMemory) -> io::Result<bool> {
        Ok(memory.load_u16(self.driver + 2)? != self.next_available)
    }

    pub fn pop(&mut self, memory: &GuestMemory) -> io::Result<Option<Chain>> {
        let available = memory.load_u16(self.driver + 2)?;
        if available == self.next_available {
            return Ok(None);
        }
        if available.wrapping_sub(self.next_available) > self.size {
            return Err(invalid("virtqueue driver ring overrun"));
        }
        let slot = self.driver + 4 + 2 * u64::from(self.next_available % self.size);
        let head = u16::from_le_bytes(read(memory, slot)?);
        self.next_available = self.next_available.wrapping_add(1);
        let mut descriptors = Vec::new();
        let mut index = head;
        loop {
            if index >= self.size || descriptors.len() == usize::from(self.size) {
                return Err(invalid("malformed virtqueue chain"));
            }
            let raw: [u8; 16] = read(memory, self.descriptors + 16 * u64::from(index))?;
            let flags = u16::from_le_bytes([raw[12], raw[13]]);
            descriptors.push(Descriptor {
                address: u64::from_le_bytes(raw[..8].try_into().unwrap()),
                len: u32::from_le_bytes(raw[8..12].try_into().unwrap()),
                writable: flags & WRITE != 0,
            });
            if flags & NEXT == 0 {
                return Ok(Some(Chain { head, descriptors }));
            }
            index = u16::from_le_bytes([raw[14], raw[15]]);
        }
    }

    /// Returns a chain to the driver with the number of bytes the device wrote into it.
    pub fn push(&mut self, memory: &GuestMemory, head: u16, written: u32) -> io::Result<()> {
        let slot = self.device + 4 + 8 * u64::from(self.next_used % self.size);
        let element = (u64::from(written) << 32) | u64::from(head);
        memory.write(slot, &element.to_le_bytes())?;
        self.next_used = self.next_used.wrapping_add(1);
        memory.store_u16(self.device + 2, self.next_used)
    }
}

impl Chain {
    pub fn readable_len(&self) -> u64 {
        self.total(false)
    }

    pub fn writable_len(&self) -> u64 {
        self.total(true)
    }

    /// Reads the readable stream from `offset`; returns how many bytes it held.
    pub fn read_at(
        &self,
        memory: &GuestMemory,
        offset: u64,
        buffer: &mut [u8],
    ) -> io::Result<usize> {
        self.each(false, offset, buffer.len(), |address, range| {
            memory.read(address, &mut buffer[range])
        })
    }

    /// Writes the writable stream from `offset`; returns how many bytes fit.
    pub fn write_at(&self, memory: &GuestMemory, offset: u64, bytes: &[u8]) -> io::Result<usize> {
        self.each(true, offset, bytes.len(), |address, range| {
            memory.write(address, &bytes[range])
        })
    }

    fn total(&self, writable: bool) -> u64 {
        self.descriptors
            .iter()
            .filter(|descriptor| descriptor.writable == writable)
            .map(|descriptor| u64::from(descriptor.len))
            .sum()
    }

    /// Calls `copy` with each guest address and buffer range overlapping `[offset, offset + len)`.
    fn each(
        &self,
        writable: bool,
        offset: u64,
        len: usize,
        mut copy: impl FnMut(u64, std::ops::Range<usize>) -> io::Result<()>,
    ) -> io::Result<usize> {
        let (mut start, mut done) = (0u64, 0usize);
        for descriptor in self.descriptors.iter().filter(|d| d.writable == writable) {
            let end = start + u64::from(descriptor.len);
            let position = offset + done as u64;
            if done < len && position < end && position >= start {
                let count = ((end - position) as usize).min(len - done);
                copy(descriptor.address + (position - start), done..done + count)?;
                done += count;
            }
            start = end;
        }
        Ok(done)
    }
}

fn read<const N: usize>(memory: &GuestMemory, address: u64) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    memory.read(address, &mut bytes)?;
    Ok(bytes)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A driver side for tests: a ring of at most 64 entries in the 4 KiB at `self.0`.
    pub struct Ring(pub u64);

    impl Ring {
        pub fn queue(&self, size: u16) -> Queue {
            Queue {
                size,
                ready: true,
                descriptors: self.0,
                driver: self.0 + 0x800,
                device: self.0 + 0xc00,
                ..Default::default()
            }
        }

        /// Writes `chain` as descriptors from `first` and offers it in driver-ring slot `slot`.
        pub fn offer(&self, memory: &GuestMemory, first: u16, slot: u16, chain: &[Descriptor]) {
            let last = first + chain.len() as u16 - 1;
            for (index, descriptor) in (first..).zip(chain) {
                let flags =
                    (u16::from(index < last) * NEXT) | (u16::from(descriptor.writable) * WRITE);
                let mut raw = [0u8; 16];
                raw[..8].copy_from_slice(&descriptor.address.to_le_bytes());
                raw[8..12].copy_from_slice(&descriptor.len.to_le_bytes());
                raw[12..14].copy_from_slice(&flags.to_le_bytes());
                raw[14..].copy_from_slice(&(index + 1).to_le_bytes());
                memory.write(self.0 + 16 * u64::from(index), &raw).unwrap();
            }
            let driver = self.0 + 0x800;
            memory
                .write(driver + 4 + 2 * u64::from(slot), &first.to_le_bytes())
                .unwrap();
            memory.store_u16(driver + 2, slot + 1).unwrap();
        }

        /// The (head, written) element the device put in used-ring slot `slot`.
        pub fn used(&self, memory: &GuestMemory, slot: u16) -> (u32, u32) {
            let element: [u8; 8] = read(memory, self.0 + 0xc00 + 4 + 8 * u64::from(slot)).unwrap();
            let (id, len) = element.split_at(4);
            (
                u32::from_le_bytes(id.try_into().unwrap()),
                u32::from_le_bytes(len.try_into().unwrap()),
            )
        }
    }

    pub fn readable(address: u64, len: u32) -> Descriptor {
        Descriptor {
            address,
            len,
            writable: false,
        }
    }

    pub fn writable(address: u64, len: u32) -> Descriptor {
        Descriptor {
            address,
            len,
            writable: true,
        }
    }

    #[test]
    fn chains_read_and_write_across_descriptors() {
        let memory = GuestMemory::map(1 << 16).unwrap();
        let ring = Ring(0x1000);
        let mut queue = ring.queue(4);
        assert!(queue.valid());
        memory.write(0x4000, b"hello").unwrap();
        memory.write(0x5000, b" world").unwrap();
        let descriptors = [
            readable(0x4000, 5),
            readable(0x5000, 6),
            writable(0x6000, 3),
            writable(0x7000, 3),
        ];
        ring.offer(&memory, 0, 0, &descriptors);
        assert!(queue.has_available(&memory).unwrap());
        let chain = queue.pop(&memory).unwrap().unwrap();
        assert_eq!(
            (chain.head, chain.descriptors.as_slice()),
            (0, &descriptors[..])
        );
        assert_eq!((chain.readable_len(), chain.writable_len()), (11, 6));
        let mut text = [0; 8];
        assert_eq!(chain.read_at(&memory, 3, &mut text).unwrap(), 8);
        assert_eq!(&text, b"lo world");
        assert_eq!(chain.write_at(&memory, 1, b"abcdefg").unwrap(), 5);
        let mut written = [0; 3];
        memory.read(0x7000, &mut written).unwrap();
        assert_eq!(&written, b"cde");
        queue.push(&memory, chain.head, 5).unwrap();
        assert_eq!(ring.used(&memory, 0), (0, 5));
        assert_eq!(memory.load_u16(0x1c02).unwrap(), 1);
        assert!(queue.pop(&memory).unwrap().is_none());
    }

    #[test]
    fn rejects_looping_and_out_of_range_chains() {
        let memory = GuestMemory::map(1 << 16).unwrap();
        let ring = Ring(0x1000);
        let mut queue = ring.queue(4);
        // Descriptor 3 points back to itself.
        ring.offer(&memory, 3, 0, &[readable(0x4000, 1)]);
        let mut raw = [0u8; 16];
        memory.read(0x1000 + 48, &mut raw).unwrap();
        raw[12] = NEXT as u8;
        raw[14..].copy_from_slice(&3u16.to_le_bytes());
        memory.write(0x1000 + 48, &raw).unwrap();
        assert!(queue.pop(&memory).is_err());
        let mut queue = ring.queue(4);
        memory.write(0x1800 + 4, &9u16.to_le_bytes()).unwrap();
        assert!(queue.pop(&memory).is_err());
        queue.size = 3;
        assert!(!queue.valid());
    }
}
