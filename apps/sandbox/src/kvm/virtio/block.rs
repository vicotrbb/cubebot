//! virtio-blk (virtio 1.2 §5.2) over a host file: the read-only root image or the workspace disk.
use super::{
    Device,
    queue::{Chain, Queue},
};
use crate::kvm::memory::{GuestMemory, invalid};
use std::{fs::File, io, os::unix::fs::FileExt};

const READ_ONLY: u64 = 1 << 5;
const FLUSH: u64 = 1 << 9;
const SECTOR: u64 = 512;
const HEADER: u64 = 16;
const CHUNK: usize = 64 << 10;
const OK: u8 = 0;
const IO_ERROR: u8 = 1;
const UNSUPPORTED: u8 = 2;

pub struct Block {
    file: File,
    read_only: bool,
    capacity: u64,
    config: [u8; 8],
}

impl Block {
    pub fn new(file: File, read_only: bool) -> io::Result<Self> {
        let capacity = file.metadata()?.len() / SECTOR * SECTOR;
        Ok(Self {
            file,
            read_only,
            capacity,
            config: (capacity / SECTOR).to_le_bytes(),
        })
    }

    /// Serves one request and returns how many bytes it wrote into the chain.
    fn serve(&self, chain: &Chain, memory: &GuestMemory) -> io::Result<u32> {
        let mut header = [0u8; HEADER as usize];
        let writable = chain.writable_len();
        if chain.read_at(memory, 0, &mut header)? < header.len() || writable == 0 {
            return Err(invalid("malformed virtio-blk request"));
        }
        let [k0, k1, k2, k3, _, _, _, _, sector @ ..] = header;
        let (kind, sector) = (
            u32::from_le_bytes([k0, k1, k2, k3]),
            u64::from_le_bytes(sector),
        );
        let status = match kind {
            0 => self.input(chain, memory, sector, writable - 1)?,
            1 => self.output(chain, memory, sector)?,
            4 => self.flush(),
            _ => UNSUPPORTED,
        };
        // The status byte is the last writable byte of every request.
        chain.write_at(memory, writable - 1, &[status])?;
        let written = if kind == 0 && status == OK {
            writable
        } else {
            1
        };
        Ok(written.try_into().unwrap_or(u32::MAX))
    }

    fn input(&self, chain: &Chain, memory: &GuestMemory, sector: u64, len: u64) -> io::Result<u8> {
        let Some(start) = self.range(sector, len) else {
            return Ok(IO_ERROR);
        };
        let mut buffer = vec![0; CHUNK];
        for done in (0..len).step_by(CHUNK) {
            let buffer = &mut buffer[..(len - done).min(CHUNK as u64) as usize];
            if let Err(error) = self.file.read_exact_at(buffer, start + done) {
                return Ok(failed(error));
            }
            chain.write_at(memory, done, buffer)?;
        }
        Ok(OK)
    }

    fn output(&self, chain: &Chain, memory: &GuestMemory, sector: u64) -> io::Result<u8> {
        let len = chain.readable_len() - HEADER;
        let Some(start) = self.range(sector, len).filter(|_| !self.read_only) else {
            return Ok(IO_ERROR);
        };
        let mut buffer = vec![0; CHUNK];
        for done in (0..len).step_by(CHUNK) {
            let buffer = &mut buffer[..(len - done).min(CHUNK as u64) as usize];
            chain.read_at(memory, HEADER + done, buffer)?;
            if let Err(error) = self.file.write_all_at(buffer, start + done) {
                return Ok(failed(error));
            }
        }
        Ok(OK)
    }

    fn flush(&self) -> u8 {
        match self.file.sync_data() {
            Ok(()) => OK,
            Err(error) => failed(error),
        }
    }

    fn range(&self, sector: u64, len: u64) -> Option<u64> {
        let start = sector.checked_mul(SECTOR)?;
        (start.checked_add(len)? <= self.capacity).then_some(start)
    }
}

/// A host I/O failure is the guest's disk error, not the sandbox's; it is logged once here.
fn failed(error: io::Error) -> u8 {
    eprintln!("Sandbox disk I/O failed: {error}");
    IO_ERROR
}

impl Device for Block {
    const TYPE: u32 = 2;
    const QUEUES: usize = 1;

    fn features(&self) -> u64 {
        FLUSH | if self.read_only { READ_ONLY } else { 0 }
    }

    fn config(&self) -> &[u8] {
        &self.config
    }

    fn notify(&mut self, _: usize, queues: &mut [Queue], memory: &GuestMemory) -> io::Result<bool> {
        let queue = &mut queues[0];
        let mut used = false;
        while let Some(chain) = queue.pop(memory)? {
            let written = self.serve(&chain, memory)?;
            queue.push(memory, chain.head, written)?;
            used = true;
        }
        Ok(used)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kvm::virtio::queue::tests::{Ring, readable, writable};

    const RING: Ring = Ring(0x1000);

    fn request(memory: &GuestMemory, kind: u32, sector: u64) {
        let mut header = [0u8; 16];
        header[..4].copy_from_slice(&kind.to_le_bytes());
        header[8..].copy_from_slice(&sector.to_le_bytes());
        memory.write(0x4000, &header).unwrap();
    }

    fn status(memory: &GuestMemory) -> u8 {
        let mut status = [0xff];
        memory.read(0x9000, &mut status).unwrap();
        status[0]
    }

    /// A four-sector disk whose name is unlinked at once.
    fn disk(name: &str) -> File {
        let path = std::env::temp_dir().join(format!("cubebot-{name}-{}", std::process::id()));
        let file = File::options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        std::fs::remove_file(path).unwrap();
        file.set_len(4 * SECTOR).unwrap();
        file
    }

    #[test]
    fn writes_reads_and_flushes_sectors() {
        let memory = GuestMemory::map(1 << 16).unwrap();
        let mut block = Block::new(disk("workspace"), false).unwrap();
        assert_eq!(block.config(), &4u64.to_le_bytes());
        assert_eq!(block.features(), FLUSH);
        let mut queues = [RING.queue(8)];
        memory.write(0x5000, &[7; 512]).unwrap();
        request(&memory, 1, 2);
        let out = [
            readable(0x4000, 16),
            readable(0x5000, 512),
            writable(0x9000, 1),
        ];
        RING.offer(&memory, 0, 0, &out);
        assert!(block.notify(0, &mut queues, &memory).unwrap());
        assert_eq!((status(&memory), RING.used(&memory, 0)), (OK, (0, 1)));
        request(&memory, 0, 2);
        let input = [
            readable(0x4000, 16),
            writable(0x6000, 512),
            writable(0x9000, 1),
        ];
        RING.offer(&memory, 3, 1, &input);
        block.notify(0, &mut queues, &memory).unwrap();
        let mut sector = [0; 512];
        memory.read(0x6000, &mut sector).unwrap();
        assert_eq!(
            (sector, status(&memory), RING.used(&memory, 1)),
            ([7; 512], OK, (3, 513))
        );
        request(&memory, 4, 0);
        RING.offer(&memory, 6, 2, &[readable(0x4000, 16), writable(0x9000, 1)]);
        block.notify(0, &mut queues, &memory).unwrap();
        assert_eq!(status(&memory), OK);
    }

    #[test]
    fn refuses_writes_to_read_only_disks_and_past_the_end() {
        let memory = GuestMemory::map(1 << 16).unwrap();
        let mut queues = [RING.queue(8)];
        let mut root = Block::new(disk("root"), true).unwrap();
        assert_eq!(root.features(), FLUSH | READ_ONLY);
        request(&memory, 1, 0);
        let out = [
            readable(0x4000, 16),
            readable(0x5000, 512),
            writable(0x9000, 1),
        ];
        RING.offer(&memory, 0, 0, &out);
        root.notify(0, &mut queues, &memory).unwrap();
        assert_eq!(status(&memory), IO_ERROR);
        request(&memory, 0, 4);
        RING.offer(
            &memory,
            3,
            1,
            &[
                readable(0x4000, 16),
                writable(0x6000, 512),
                writable(0x9000, 1),
            ],
        );
        root.notify(0, &mut queues, &memory).unwrap();
        assert_eq!((status(&memory), RING.used(&memory, 1)), (IO_ERROR, (3, 1)));
        request(&memory, 8, 0);
        RING.offer(&memory, 6, 2, &[readable(0x4000, 16), writable(0x9000, 1)]);
        root.notify(0, &mut queues, &memory).unwrap();
        assert_eq!(status(&memory), UNSUPPORTED);
    }
}
