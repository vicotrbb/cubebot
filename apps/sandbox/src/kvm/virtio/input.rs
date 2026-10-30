//! virtio-input (virtio 1.2 §5.8): the guest's keyboard and absolute tablet (ADR 0085). The host
//! queues evdev events with [`Input::send`]; the guest's status events, such as LEDs, are dropped.
use super::{Device, queue::Queue};
use crate::kvm::memory::GuestMemory;
use std::{collections::VecDeque, io};

const EVENT: usize = 0;
const CONFIG: usize = 136;
const ID_NAME: u8 = 0x01;
const EV_BITS: u8 = 0x11;
const ABS_INFO: u8 = 0x12;
const EV_KEY: u8 = 1;
const EV_REL: u8 = 2;
const EV_ABS: u8 = 3;
/// REL_HWHEEL and REL_WHEEL, a tablet's scroll wheels.
const WHEELS: [u16; 2] = [6, 8];
const BUTTONS: [u16; 3] = [0x110, 0x111, 0x112]; // BTN_LEFT, BTN_RIGHT, BTN_MIDDLE

pub struct Input {
    name: &'static [u8],
    keys: Vec<u16>,
    /// ABS_X and ABS_Y maxima, for a tablet.
    axes: Option<[u32; 2]>,
    /// select, subsel, size, five reserved bytes and the selected 128-byte payload.
    config: [u8; CONFIG],
    pending: VecDeque<[u8; 8]>,
}

impl Input {
    /// A keyboard with every key code from KEY_ESC to 255.
    pub fn keyboard() -> Self {
        Self::new(b"cubebot keyboard", (1..=255).collect(), None)
    }

    /// A pointer with three buttons and scroll wheels whose ABS_X and ABS_Y span a `width` by
    /// `height` screen.
    pub fn tablet(width: u32, height: u32) -> Self {
        Self::new(
            b"cubebot tablet",
            BUTTONS.to_vec(),
            Some([width - 1, height - 1]),
        )
    }

    fn new(name: &'static [u8], keys: Vec<u16>, axes: Option<[u32; 2]>) -> Self {
        Self {
            name,
            keys,
            axes,
            config: [0; CONFIG],
            pending: VecDeque::new(),
        }
    }

    /// Queues evdev (type, code, value) events and delivers what the guest's buffers hold.
    pub fn send(
        &mut self,
        events: &[(u16, u16, u32)],
        queues: &mut [Queue],
        memory: &GuestMemory,
    ) -> io::Result<bool> {
        self.pending
            .extend(events.iter().map(|&(kind, code, value)| {
                let mut event = [0u8; 8];
                event[..2].copy_from_slice(&kind.to_le_bytes());
                event[2..4].copy_from_slice(&code.to_le_bytes());
                event[4..].copy_from_slice(&value.to_le_bytes());
                event
            }));
        self.deliver(queues, memory)
    }

    fn deliver(&mut self, queues: &mut [Queue], memory: &GuestMemory) -> io::Result<bool> {
        let mut used = false;
        while let Some(event) = self.pending.front() {
            let Some(chain) = queues[EVENT].pop(memory)? else {
                break;
            };
            let written = chain.write_at(memory, 0, event)?;
            queues[EVENT].push(memory, chain.head, written as u32)?;
            self.pending.pop_front();
            used = true;
        }
        Ok(used)
    }

    /// The payload the driver's select and subsel name; empty for what the device lacks.
    fn selected(&self) -> Vec<u8> {
        match (self.config[0], self.config[1], self.axes) {
            (ID_NAME, 0, _) => self.name.to_vec(),
            (EV_BITS, EV_KEY, _) => bitmap(&self.keys),
            (EV_BITS, EV_REL, Some(_)) => bitmap(&WHEELS),
            (EV_BITS, EV_ABS, Some(_)) => bitmap(&[0, 1]),
            (ABS_INFO, axis @ 0..=1, Some(axes)) => {
                // min, max, fuzz, flat and resolution.
                let info = [0, axes[usize::from(axis)], 0, 0, 0];
                info.iter().flat_map(|value| value.to_le_bytes()).collect()
            }
            _ => Vec::new(),
        }
    }
}

/// The smallest little-endian bitmap with `bits` set.
fn bitmap(bits: &[u16]) -> Vec<u8> {
    let mut bytes = vec![0u8; bits.iter().max().map_or(0, |&bit| usize::from(bit) / 8 + 1)];
    for &bit in bits {
        bytes[usize::from(bit) / 8] |= 1 << (bit % 8);
    }
    bytes
}

impl Device for Input {
    const TYPE: u32 = 18;
    /// The event queue and the status queue.
    const QUEUES: usize = 2;

    fn features(&self) -> u64 {
        0
    }

    fn config(&self) -> &[u8] {
        &self.config
    }

    /// Only select and subsel are writable; writing either selects a new payload.
    fn write_config(&mut self, offset: u64, data: &[u8]) {
        for (at, &byte) in (offset..2).zip(data) {
            self.config[at as usize] = byte;
        }
        let payload = self.selected();
        self.config[2] = payload.len() as u8;
        self.config[8..].fill(0);
        self.config[8..8 + payload.len()].copy_from_slice(&payload);
    }

    fn notify(
        &mut self,
        index: usize,
        queues: &mut [Queue],
        memory: &GuestMemory,
    ) -> io::Result<bool> {
        if index == EVENT {
            return self.deliver(queues, memory);
        }
        let mut used = false;
        while let Some(chain) = queues[index].pop(memory)? {
            queues[index].push(memory, chain.head, 0)?;
            used = true;
        }
        Ok(used)
    }

    fn reset(&mut self) {
        self.config = [0; CONFIG];
        self.pending.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kvm::virtio::queue::tests::{Ring, writable};

    fn select(input: &mut Input, select: u8, subsel: u8) -> Vec<u8> {
        input.write_config(0, &[select]);
        input.write_config(1, &[subsel]);
        let size = usize::from(input.config()[2]);
        input.config()[8..8 + size].to_vec()
    }

    #[test]
    fn describes_a_keyboard_and_a_tablet() {
        let mut keyboard = Input::keyboard();
        assert_eq!(select(&mut keyboard, ID_NAME, 0), b"cubebot keyboard");
        let keys = select(&mut keyboard, EV_BITS, EV_KEY);
        assert_eq!((keys.len(), keys[0], keys[3]), (32, 0xfe, 0xff));
        assert!(select(&mut keyboard, EV_BITS, EV_ABS).is_empty());
        let mut tablet = Input::tablet(1280, 800);
        assert_eq!(select(&mut tablet, EV_BITS, EV_ABS), [0b11]);
        assert_eq!(select(&mut tablet, EV_BITS, EV_REL), [0b0100_0000, 0b1]);
        assert!(select(&mut keyboard, EV_BITS, EV_REL).is_empty());
        assert_eq!(select(&mut tablet, EV_BITS, EV_KEY)[34], 0b111);
        let y = select(&mut tablet, ABS_INFO, 1);
        assert_eq!(u32::from_le_bytes(y[4..8].try_into().unwrap()), 799);
        assert!(select(&mut tablet, ABS_INFO, 2).is_empty());
        assert!(select(&mut tablet, 0x7f, 0).is_empty());
    }

    #[test]
    fn delivers_events_as_the_guest_posts_buffers() {
        let memory = GuestMemory::map(1 << 16).unwrap();
        let ring = Ring(0x1000);
        let mut queues = [ring.queue(8), Ring(0x2000).queue(8)];
        let mut keyboard = Input::keyboard();
        let press = [(1, 30, 1), (0, 0, 0)];
        ring.offer(&memory, 0, 0, &[writable(0x4000, 8)]);
        assert!(keyboard.send(&press, &mut queues, &memory).unwrap());
        assert_eq!(keyboard.pending.len(), 1, "one buffer, so the SYN waits");
        let mut event = [0u8; 8];
        memory.read(0x4000, &mut event).unwrap();
        assert_eq!(event, [1, 0, 30, 0, 1, 0, 0, 0]);
        ring.offer(&memory, 1, 1, &[writable(0x5000, 8)]);
        assert!(keyboard.notify(EVENT, &mut queues, &memory).unwrap());
        assert_eq!((ring.used(&memory, 1), keyboard.pending.len()), ((1, 8), 0));
    }
}
