//! virtio-gpu (virtio 1.2 §5.7) with the 2D command set only (ADR 0085). The guest's resources
//! hold host copies of its pixels, and flushing the scanned-out one copies it into the one
//! 1280x800 BGRX screen the host keeps, recording the flushed area as dirty.
use super::{
    Device,
    queue::{Chain, Queue},
};
use crate::kvm::memory::{GuestMemory, invalid};
use std::{collections::HashMap, io};

pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 800;
const PIXEL: usize = 4;
const HEADER: usize = 24;
/// Bounds what guest-supplied sizes make the host hold: all resources' pixels, and each backing.
const RESOURCE_LIMIT: u64 = 256 << 20;
const BACKING_LIMIT: u32 = 16384;
/// Past this many dirty rects, one rect covering the screen replaces them.
const DIRTY_LIMIT: usize = 64;
const SCREEN: Rect = Rect {
    x: 0,
    y: 0,
    width: WIDTH,
    height: HEIGHT,
};
// Commands and responses (§5.7.6.7).
const GET_DISPLAY_INFO: u32 = 0x0100;
const RESOURCE_CREATE_2D: u32 = 0x0101;
const RESOURCE_UNREF: u32 = 0x0102;
const SET_SCANOUT: u32 = 0x0103;
const RESOURCE_FLUSH: u32 = 0x0104;
const TRANSFER_TO_HOST_2D: u32 = 0x0105;
const RESOURCE_ATTACH_BACKING: u32 = 0x0106;
const RESOURCE_DETACH_BACKING: u32 = 0x0107;
const OK_NODATA: u32 = 0x1100;
const OK_DISPLAY_INFO: u32 = 0x1101;
const ERR_UNSPEC: u32 = 0x1200;
const ERR_OUT_OF_MEMORY: u32 = 0x1201;
const ERR_INVALID_SCANOUT_ID: u32 = 0x1202;
const ERR_INVALID_RESOURCE_ID: u32 = 0x1203;
const ERR_INVALID_PARAMETER: u32 = 0x1205;
/// The cursor's and the screen's formats; Linux uses no others for 2D.
const FORMATS: [u32; 2] = [1, 2];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

struct Resource {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    /// Guest physical (address, length) entries, already checked to lie in guest memory.
    backing: Vec<(u64, u32)>,
}

impl Resource {
    fn holds(&self, rect: Rect) -> bool {
        rect.x
            .checked_add(rect.width)
            .is_some_and(|end| end <= self.width)
            && rect
                .y
                .checked_add(rect.height)
                .is_some_and(|end| end <= self.height)
    }
}

pub struct Gpu {
    /// The screen as the guest last flushed it: BGRX rows of WIDTH pixels.
    pub screen: Vec<u8>,
    /// Screen areas flushed since the arbiter last took them.
    pub dirty: Vec<Rect>,
    resources: HashMap<u32, Resource>,
    /// The scanned-out resource and the area of it shown at the screen's origin.
    scanout: Option<(u32, Rect)>,
    held: u64,
}

impl Default for Gpu {
    fn default() -> Self {
        Self {
            screen: vec![0; WIDTH as usize * HEIGHT as usize * PIXEL],
            dirty: Vec::new(),
            resources: HashMap::new(),
            scanout: None,
            held: 0,
        }
    }
}

impl Gpu {
    /// Serves one control request and returns how many bytes of response it wrote.
    fn serve(&mut self, chain: &Chain, memory: &GuestMemory) -> io::Result<u32> {
        // The longest fixed-size request, TRANSFER_TO_HOST_2D; missing fields read as zero.
        let mut request = [0u8; 56];
        if chain.read_at(memory, 0, &mut request)? < HEADER {
            return Err(invalid("malformed virtio-gpu request"));
        }
        let (status, display) = match word(&request, 0) {
            GET_DISPLAY_INFO => (OK_DISPLAY_INFO, true),
            RESOURCE_ATTACH_BACKING => (self.attach(chain, memory, &request)?, false),
            TRANSFER_TO_HOST_2D => (self.transfer(memory, &request)?, false),
            _ => (self.command(&request), false),
        };
        // The response keeps the request's fence flag, fence and context (§5.7.6.7).
        let mut response = request[..HEADER].to_vec();
        response[..4].copy_from_slice(&status.to_le_bytes());
        if display {
            response.extend(display_info());
        }
        let written = chain.write_at(memory, 0, &response)?;
        Ok(written as u32)
    }

    fn command(&mut self, request: &[u8]) -> u32 {
        match word(request, 0) {
            RESOURCE_CREATE_2D => self.create(
                word(request, 24),
                word(request, 28),
                word(request, 32),
                word(request, 36),
            ),
            RESOURCE_UNREF => self.unref(word(request, 24)),
            SET_SCANOUT => self.set_scanout(word(request, 40), word(request, 44), rect(request)),
            RESOURCE_FLUSH => self.flush(word(request, 40), rect(request)),
            RESOURCE_DETACH_BACKING => match self.resources.get_mut(&word(request, 24)) {
                Some(resource) => {
                    resource.backing.clear();
                    OK_NODATA
                }
                None => ERR_INVALID_RESOURCE_ID,
            },
            _ => ERR_UNSPEC,
        }
    }

    fn create(&mut self, id: u32, format: u32, width: u32, height: u32) -> u32 {
        if id == 0 || self.resources.contains_key(&id) {
            return ERR_INVALID_RESOURCE_ID;
        }
        let size = (u64::from(width) * u64::from(height)).saturating_mul(PIXEL as u64);
        if !FORMATS.contains(&format) || size == 0 {
            return ERR_INVALID_PARAMETER;
        }
        if self.held + size > RESOURCE_LIMIT {
            return ERR_OUT_OF_MEMORY;
        }
        self.held += size;
        let resource = Resource {
            width,
            height,
            pixels: vec![0; size as usize],
            backing: Vec::new(),
        };
        self.resources.insert(id, resource);
        OK_NODATA
    }

    fn unref(&mut self, id: u32) -> u32 {
        let Some(resource) = self.resources.remove(&id) else {
            return ERR_INVALID_RESOURCE_ID;
        };
        self.held -= resource.pixels.len() as u64;
        if self.scanout.is_some_and(|(shown, _)| shown == id) {
            self.scanout = None;
        }
        OK_NODATA
    }

    fn attach(&mut self, chain: &Chain, memory: &GuestMemory, request: &[u8]) -> io::Result<u32> {
        let Some(resource) = self.resources.get_mut(&word(request, 24)) else {
            return Ok(ERR_INVALID_RESOURCE_ID);
        };
        let count = word(request, 28);
        let mut entries = vec![0u8; count.min(BACKING_LIMIT) as usize * 16];
        if count > BACKING_LIMIT || chain.read_at(memory, 32, &mut entries)? < entries.len() {
            return Ok(ERR_INVALID_PARAMETER);
        }
        let backing: Vec<_> = entries
            .chunks(16)
            .map(|entry| {
                (
                    u64::from_le_bytes(entry[..8].try_into().unwrap()),
                    word(entry, 8),
                )
            })
            .collect();
        let inside = backing.iter().all(|&(address, len)| {
            address
                .checked_add(len.into())
                .is_some_and(|end| end <= memory.size() as u64)
        });
        if !inside {
            return Ok(ERR_INVALID_PARAMETER);
        }
        resource.backing = backing;
        Ok(OK_NODATA)
    }

    /// Copies a rect of the guest's backing, whose rows start at `offset` one resource row apart,
    /// into the resource.
    fn transfer(&mut self, memory: &GuestMemory, request: &[u8]) -> io::Result<u32> {
        let (area, offset) = (
            rect(request),
            u64::from_le_bytes(request[40..48].try_into().unwrap()),
        );
        let Some(resource) = self.resources.get_mut(&word(request, 48)) else {
            return Ok(ERR_INVALID_RESOURCE_ID);
        };
        if !resource.holds(area) {
            return Ok(ERR_INVALID_PARAMETER);
        }
        let stride = resource.width as usize * PIXEL;
        for row in 0..area.height as usize {
            let start = (area.y as usize + row) * stride + area.x as usize * PIXEL;
            let line = &mut resource.pixels[start..start + area.width as usize * PIXEL];
            let Some(source) = offset.checked_add((row * stride) as u64) else {
                return Ok(ERR_INVALID_PARAMETER);
            };
            if !read_backing(&resource.backing, memory, source, line)? {
                return Ok(ERR_INVALID_PARAMETER);
            }
        }
        Ok(OK_NODATA)
    }

    fn set_scanout(&mut self, scanout: u32, id: u32, area: Rect) -> u32 {
        if scanout != 0 {
            return ERR_INVALID_SCANOUT_ID;
        }
        if id == 0 {
            self.scanout = None;
            return OK_NODATA;
        }
        match self.resources.get(&id) {
            Some(resource) if resource.holds(area) => {
                self.scanout = Some((id, area));
                OK_NODATA
            }
            Some(_) => ERR_INVALID_PARAMETER,
            None => ERR_INVALID_RESOURCE_ID,
        }
    }

    /// Copies the flushed part of the scanned-out area onto the screen and marks it dirty.
    fn flush(&mut self, id: u32, area: Rect) -> u32 {
        let Some(resource) = self.resources.get(&id) else {
            return ERR_INVALID_RESOURCE_ID;
        };
        if !resource.holds(area) {
            return ERR_INVALID_PARAMETER;
        }
        let Some((_, shown)) = self.scanout.filter(|&(shown, _)| shown == id) else {
            return OK_NODATA;
        };
        let left = area.x.max(shown.x);
        let top = area.y.max(shown.y);
        let right = (area.x + area.width).min(shown.x + shown.width.min(WIDTH));
        let bottom = (area.y + area.height).min(shown.y + shown.height.min(HEIGHT));
        if left >= right || top >= bottom {
            return OK_NODATA;
        }
        let len = (right - left) as usize * PIXEL;
        for y in top..bottom {
            let from = (y as usize * resource.width as usize + left as usize) * PIXEL;
            let to = ((y - shown.y) as usize * WIDTH as usize + (left - shown.x) as usize) * PIXEL;
            self.screen[to..to + len].copy_from_slice(&resource.pixels[from..from + len]);
        }
        self.dirty.push(Rect {
            x: left - shown.x,
            y: top - shown.y,
            width: right - left,
            height: bottom - top,
        });
        if self.dirty.len() > DIRTY_LIMIT {
            self.dirty = vec![SCREEN];
        }
        OK_NODATA
    }
}

impl Device for Gpu {
    const TYPE: u32 = 16;
    /// The control queue and the cursor queue.
    const QUEUES: usize = 2;

    fn features(&self) -> u64 {
        0
    }

    /// events_read, events_clear, one scanout and no capability sets.
    fn config(&self) -> &[u8] {
        &[0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0]
    }

    fn notify(
        &mut self,
        index: usize,
        queues: &mut [Queue],
        memory: &GuestMemory,
    ) -> io::Result<bool> {
        let queue = &mut queues[index];
        let mut used = false;
        while let Some(chain) = queue.pop(memory)? {
            // shortcut: cursor updates are dropped, so screenshots lack the pointer; draw it when
            // the arbiter needs it.
            let written = match index {
                0 => self.serve(&chain, memory)?,
                _ => 0,
            };
            queue.push(memory, chain.head, written)?;
            used = true;
        }
        Ok(used)
    }

    fn reset(&mut self) {
        self.resources.clear();
        self.scanout = None;
        self.held = 0;
    }
}

fn word(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap())
}

/// The rect every 2D command with one carries right after the header.
fn rect(request: &[u8]) -> Rect {
    Rect {
        x: word(request, 24),
        y: word(request, 28),
        width: word(request, 32),
        height: word(request, 36),
    }
}

/// The one enabled scanout of VIRTIO_GPU_MAX_SCANOUTS (16).
fn display_info() -> Vec<u8> {
    let mut modes = vec![0u8; 16 * 24];
    for (at, value) in [(8, WIDTH), (12, HEIGHT), (16, 1)] {
        modes[at..at + 4].copy_from_slice(&value.to_le_bytes());
    }
    modes
}

/// Reads `buffer.len()` bytes from `offset` in a backing; returns whether the backing held them.
fn read_backing(
    backing: &[(u64, u32)],
    memory: &GuestMemory,
    offset: u64,
    buffer: &mut [u8],
) -> io::Result<bool> {
    let (mut start, mut done) = (0u64, 0usize);
    for &(address, len) in backing {
        let end = start + u64::from(len);
        let position = offset.saturating_add(done as u64);
        if done < buffer.len() && position >= start && position < end {
            let count = ((end - position) as usize).min(buffer.len() - done);
            memory.read(
                address + (position - start),
                &mut buffer[done..done + count],
            )?;
            done += count;
        }
        start = end;
    }
    Ok(done == buffer.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kvm::virtio::queue::tests::{Ring, readable, writable};

    /// The driver side of the control queue: each request in its own slot of one ring.
    struct Driver {
        memory: GuestMemory,
        queues: [Queue; 2],
        slot: u16,
    }

    impl Driver {
        fn new() -> Self {
            Self {
                memory: GuestMemory::map(1 << 24).unwrap(),
                queues: [Ring(0x1000).queue(64), Ring(0x2000).queue(64)],
                slot: 0,
            }
        }

        /// Sends one request of little-endian words and returns the response type.
        fn send(&mut self, gpu: &mut Gpu, request: &[u32]) -> u32 {
            let mut bytes = vec![0u8; HEADER];
            bytes[..4].copy_from_slice(&request[0].to_le_bytes());
            bytes[8..16].copy_from_slice(&7u64.to_le_bytes());
            bytes.extend(request[1..].iter().flat_map(|word| word.to_le_bytes()));
            self.memory.write(0x4000, &bytes).unwrap();
            let chain = [readable(0x4000, bytes.len() as u32), writable(0x8000, 408)];
            Ring(0x1000).offer(&self.memory, self.slot * 2, self.slot, &chain);
            self.slot += 1;
            assert!(gpu.notify(0, &mut self.queues, &self.memory).unwrap());
            let mut response = [0u8; 16];
            self.memory.read(0x8000, &mut response).unwrap();
            assert_eq!(response[8], 7, "the response keeps the fence");
            word(&response, 0)
        }
    }

    #[test]
    fn transfers_and_flushes_the_scanout_onto_the_screen() {
        let (mut driver, mut gpu) = (Driver::new(), Gpu::default());
        assert_eq!(driver.send(&mut gpu, &[GET_DISPLAY_INFO]), OK_DISPLAY_INFO);
        let mut info = [0u8; 12];
        driver
            .memory
            .read(0x8000 + HEADER as u64 + 8, &mut info)
            .unwrap();
        assert_eq!(
            [word(&info, 0), word(&info, 4), word(&info, 8)],
            [WIDTH, HEIGHT, 1]
        );
        // A 2x2 resource whose rows lie on two guest pages.
        driver
            .memory
            .write(0x10000, &[1, 2, 3, 4, 5, 6, 7, 8])
            .unwrap();
        driver
            .memory
            .write(0x20000, &[9, 10, 11, 12, 13, 14, 15, 16])
            .unwrap();
        let requests: [&[u32]; 4] = [
            &[RESOURCE_CREATE_2D, 5, 2, 2, 2],
            &[
                RESOURCE_ATTACH_BACKING,
                5,
                2,
                0x10000,
                0,
                8,
                0,
                0x20000,
                0,
                8,
                0,
            ],
            &[TRANSFER_TO_HOST_2D, 0, 0, 2, 2, 0, 0, 5, 0],
            &[SET_SCANOUT, 0, 0, 2, 2, 0, 5],
        ];
        for request in requests {
            assert_eq!(driver.send(&mut gpu, request), OK_NODATA);
        }
        // Flushing the right column puts only it on the screen.
        assert_eq!(
            driver.send(&mut gpu, &[RESOURCE_FLUSH, 1, 0, 1, 2, 5, 0]),
            OK_NODATA
        );
        let row = WIDTH as usize * PIXEL;
        assert_eq!(&gpu.screen[..8], &[0, 0, 0, 0, 5, 6, 7, 8]);
        assert_eq!(&gpu.screen[row..row + 8], &[0, 0, 0, 0, 13, 14, 15, 16]);
        let dirty = Rect {
            x: 1,
            y: 0,
            width: 1,
            height: 2,
        };
        assert_eq!(gpu.dirty, [dirty]);
        assert_eq!(driver.send(&mut gpu, &[RESOURCE_UNREF, 5, 0]), OK_NODATA);
        assert_eq!((gpu.held, gpu.scanout), (0, None));
    }

    #[test]
    fn refuses_bad_ids_sizes_and_backing() {
        let (mut driver, mut gpu) = (Driver::new(), Gpu::default());
        let refused: [(&[u32], u32); 10] = [
            (&[RESOURCE_CREATE_2D, 0, 2, 1, 1], ERR_INVALID_RESOURCE_ID),
            (
                &[RESOURCE_CREATE_2D, 1, 2, 1 << 16, 1 << 16],
                ERR_OUT_OF_MEMORY,
            ),
            (&[RESOURCE_CREATE_2D, 1, 99, 1, 1], ERR_INVALID_PARAMETER),
            (&[RESOURCE_CREATE_2D, 1, 2, 4, 4], OK_NODATA),
            // Backing past the end of guest memory.
            (
                &[RESOURCE_ATTACH_BACKING, 1, 1, 0xff_f000, 0, 0x2000, 0],
                ERR_INVALID_PARAMETER,
            ),
            // No backing to transfer from.
            (
                &[TRANSFER_TO_HOST_2D, 0, 0, 4, 4, 0, 0, 1, 0],
                ERR_INVALID_PARAMETER,
            ),
            (&[RESOURCE_FLUSH, 2, 2, 3, 1, 1, 0], ERR_INVALID_PARAMETER),
            (&[SET_SCANOUT, 0, 0, 4, 4, 1, 1], ERR_INVALID_SCANOUT_ID),
            (&[RESOURCE_UNREF, 9, 0], ERR_INVALID_RESOURCE_ID),
            // A 3D command.
            (&[0x0200], ERR_UNSPEC),
        ];
        for (request, status) in refused {
            assert_eq!(driver.send(&mut gpu, request), status, "{request:x?}");
        }
    }
}
