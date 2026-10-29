//! Virtio over MMIO (virtio 1.2 §4.2, version 2 registers). A [`Transport`] holds the registers
//! and virtqueues of one [`Device`]; devices only see their queues and guest memory.
pub mod block;
pub mod net;
pub mod queue;
pub mod vsock;
use super::{Irq, memory::GuestMemory};
use queue::Queue;
use std::{
    io,
    sync::{Mutex, MutexGuard},
};

pub const WINDOW: u64 = 0x1000;
const MAGIC: u32 = 0x7472_6976; // "virt"
const VERSION_1: u64 = 1 << 32;
const FEATURES_OK: u32 = 8;
const DRIVER_OK: u32 = 4;
const USED_BUFFER: u32 = 1;
const CONFIG: u64 = 0x100;

pub trait Device: Send {
    const TYPE: u32;
    const QUEUES: usize;
    /// Device-specific feature bits; the transport adds VIRTIO_F_VERSION_1.
    fn features(&self) -> u64;
    /// Read-only device configuration space.
    fn config(&self) -> &[u8];
    /// Serves the driver's notification for `queue`; returns whether the device used buffers.
    fn notify(
        &mut self,
        queue: usize,
        queues: &mut [Queue],
        memory: &GuestMemory,
    ) -> io::Result<bool>;
    /// The driver reset the device.
    fn reset(&mut self) {}
}

/// A device's MMIO window as the vCPU sees it.
pub trait Mmio: Send + Sync {
    fn read(&self, offset: u64, data: &mut [u8]);
    fn write(&self, offset: u64, data: &[u8], memory: &GuestMemory) -> io::Result<()>;
}

/// One device on the bus: its transport, shared by the vCPU and device threads, and its interrupt.
pub struct Slot<D> {
    transport: Mutex<Transport<D>>,
    pub irq: Irq,
}

impl<D: Device> Slot<D> {
    pub fn new(device: D, irq: Irq) -> Self {
        Self {
            transport: Mutex::new(Transport::new(device)),
            irq,
        }
    }

    pub fn lock(&self) -> MutexGuard<'_, Transport<D>> {
        self.transport.lock().expect("device lock")
    }

    /// Runs device work outside a driver notification and interrupts the guest if it used buffers.
    pub fn run(
        &self,
        work: impl FnOnce(&mut D, &mut [Queue]) -> io::Result<bool>,
    ) -> io::Result<()> {
        if self.lock().run(work)? {
            self.irq.raise()?;
        }
        Ok(())
    }
}

impl<D: Device> Mmio for Slot<D> {
    fn read(&self, offset: u64, data: &mut [u8]) {
        self.lock().read(offset, data);
    }

    fn write(&self, offset: u64, data: &[u8], memory: &GuestMemory) -> io::Result<()> {
        if self.lock().write(offset, data, memory)? {
            self.irq.raise()?;
        }
        Ok(())
    }
}

pub struct Transport<D> {
    pub device: D,
    queues: Vec<Queue>,
    status: u32,
    interrupt: u32,
    device_features_select: u32,
    driver_features_select: u32,
    driver_features: u64,
    queue_select: u32,
}

impl<D: Device> Transport<D> {
    pub fn new(device: D) -> Self {
        Self {
            device,
            queues: (0..D::QUEUES).map(|_| Queue::default()).collect(),
            status: 0,
            interrupt: 0,
            device_features_select: 0,
            driver_features_select: 0,
            driver_features: 0,
            queue_select: 0,
        }
    }

    pub fn read(&self, offset: u64, data: &mut [u8]) {
        if offset >= CONFIG {
            let config = self.device.config();
            for (index, byte) in data.iter_mut().enumerate() {
                let at = usize::try_from(offset - CONFIG).unwrap_or(usize::MAX);
                *byte = at
                    .checked_add(index)
                    .and_then(|at| config.get(at))
                    .copied()
                    .unwrap_or(0);
            }
            return;
        }
        let value = self.register(offset);
        for (byte, value) in data.iter_mut().zip(value.to_le_bytes()) {
            *byte = value;
        }
    }

    /// Returns whether the write made the device use buffers, so the caller raises its interrupt.
    pub fn write(&mut self, offset: u64, data: &[u8], memory: &GuestMemory) -> io::Result<bool> {
        // Registers are 32-bit; the configuration spaces here are read-only.
        let Ok(bytes) = <[u8; 4]>::try_from(data) else {
            return Ok(false);
        };
        let value = u32::from_le_bytes(bytes);
        match offset {
            0x014 => self.device_features_select = value,
            0x020 => self.set_driver_features(value),
            0x024 => self.driver_features_select = value,
            0x030 => self.queue_select = value,
            0x050 => return self.notify(value as usize, memory),
            0x064 => self.interrupt &= !value,
            0x070 => self.set_status(value),
            _ => self.write_queue(offset, value),
        }
        Ok(false)
    }

    /// Runs device work that does not come from a driver notification, such as a received frame.
    pub fn run(
        &mut self,
        work: impl FnOnce(&mut D, &mut [Queue]) -> io::Result<bool>,
    ) -> io::Result<bool> {
        if self.status & DRIVER_OK == 0 {
            return Ok(false);
        }
        let used = work(&mut self.device, &mut self.queues)?;
        self.interrupt |= u32::from(used) * USED_BUFFER;
        Ok(used)
    }

    fn register(&self, offset: u64) -> u32 {
        let queue = self.selected();
        match offset {
            0x000 => MAGIC,
            0x004 => 2,
            0x008 => D::TYPE,
            0x010 => match self.device_features_select {
                0 => self.features() as u32,
                1 => (self.features() >> 32) as u32,
                _ => 0,
            },
            0x034 => queue.map_or(0, |_| queue::MAX_SIZE.into()),
            0x044 => queue.is_some_and(|queue| queue.ready).into(),
            0x060 => self.interrupt,
            0x070 => self.status,
            // 0x0fc ConfigGeneration stays 0: no configuration space here ever changes.
            _ => 0,
        }
    }

    fn features(&self) -> u64 {
        VERSION_1 | self.device.features()
    }

    fn set_driver_features(&mut self, value: u32) {
        match self.driver_features_select {
            0 => self.driver_features = self.driver_features & !0xffff_ffff | u64::from(value),
            1 => self.driver_features = self.driver_features & 0xffff_ffff | u64::from(value) << 32,
            _ => (),
        }
    }

    fn set_status(&mut self, value: u32) {
        if value == 0 {
            self.reset();
            return;
        }
        let acceptable =
            self.driver_features & !self.features() == 0 && self.driver_features & VERSION_1 != 0;
        // Refusing FEATURES_OK is how a device rejects the driver's feature set (§3.1.1).
        self.status = if acceptable {
            value
        } else {
            value & !FEATURES_OK
        };
    }

    fn reset(&mut self) {
        self.queues.fill_with(Queue::default);
        self.status = 0;
        self.interrupt = 0;
        self.device_features_select = 0;
        self.driver_features_select = 0;
        self.driver_features = 0;
        self.queue_select = 0;
        self.device.reset();
    }

    fn notify(&mut self, queue: usize, memory: &GuestMemory) -> io::Result<bool> {
        let ready = self.queues.get(queue).is_some_and(|queue| queue.ready);
        if !ready || self.status & DRIVER_OK == 0 {
            return Ok(false);
        }
        let used = self.device.notify(queue, &mut self.queues, memory)?;
        self.interrupt |= u32::from(used) * USED_BUFFER;
        Ok(used)
    }

    fn write_queue(&mut self, offset: u64, value: u32) {
        let Some(queue) = self.queues.get_mut(self.queue_select as usize) else {
            return;
        };
        let low = |address: u64| address & !0xffff_ffff | u64::from(value);
        let high = |address: u64| address & 0xffff_ffff | u64::from(value) << 32;
        match offset {
            0x038 => queue.size = value.try_into().unwrap_or(0),
            0x044 => queue.ready = value == 1 && queue.valid(),
            0x080 => queue.descriptors = low(queue.descriptors),
            0x084 => queue.descriptors = high(queue.descriptors),
            0x090 => queue.driver = low(queue.driver),
            0x094 => queue.driver = high(queue.driver),
            0x0a0 => queue.device = low(queue.device),
            0x0a4 => queue.device = high(queue.device),
            _ => (),
        }
    }

    fn selected(&self) -> Option<&Queue> {
        self.queues.get(self.queue_select as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Counter(u32);

    impl Device for Counter {
        const TYPE: u32 = 99;
        const QUEUES: usize = 1;

        fn features(&self) -> u64 {
            1
        }

        fn config(&self) -> &[u8] {
            b"cfg"
        }

        fn notify(&mut self, _: usize, _: &mut [Queue], _: &GuestMemory) -> io::Result<bool> {
            self.0 += 1;
            Ok(true)
        }

        fn reset(&mut self) {
            self.0 = 0;
        }
    }

    fn read(transport: &Transport<Counter>, offset: u64) -> u32 {
        let mut data = [0; 4];
        transport.read(offset, &mut data);
        u32::from_le_bytes(data)
    }

    fn write(transport: &mut Transport<Counter>, offset: u64, value: u32) -> bool {
        let memory = GuestMemory::map(4096).unwrap();
        transport
            .write(offset, &value.to_le_bytes(), &memory)
            .unwrap()
    }

    /// A transport whose driver negotiated VERSION_1 and the device's own feature.
    fn negotiated() -> Transport<Counter> {
        let mut transport = Transport::new(Counter(0));
        write(&mut transport, 0x20, 1);
        write(&mut transport, 0x24, 1);
        write(&mut transport, 0x20, 1);
        write(&mut transport, 0x70, 1 | 2 | FEATURES_OK);
        transport
    }

    #[test]
    fn identifies_the_device_and_negotiates_features() {
        let mut transport = Transport::new(Counter(0));
        assert_eq!(
            [
                read(&transport, 0),
                read(&transport, 4),
                read(&transport, 8)
            ],
            [MAGIC, 2, 99]
        );
        assert_eq!(read(&transport, 0x10), 1);
        write(&mut transport, 0x14, 1);
        assert_eq!(read(&transport, 0x10), 1);
        let mut config = [0; 4];
        transport.read(0x101, &mut config);
        assert_eq!(&config, b"fg\0\0");
        // Without VIRTIO_F_VERSION_1 the device refuses FEATURES_OK.
        write(&mut transport, 0x20, 1);
        write(&mut transport, 0x70, 1 | 2 | FEATURES_OK);
        assert_eq!(read(&transport, 0x70), 3);
        assert_eq!(read(&negotiated(), 0x70), 3 | FEATURES_OK);
    }

    #[test]
    fn notifies_ready_queues_after_driver_ok_until_reset() {
        let mut transport = negotiated();
        assert_eq!(read(&transport, 0x34), u32::from(queue::MAX_SIZE));
        write(&mut transport, 0x38, 3);
        write(&mut transport, 0x44, 1);
        assert_eq!(
            read(&transport, 0x44),
            0,
            "a size that is not a power of two"
        );
        write(&mut transport, 0x38, 8);
        write(&mut transport, 0x44, 1);
        assert_eq!(read(&transport, 0x44), 1);
        assert!(
            !write(&mut transport, 0x50, 0),
            "no notifications before DRIVER_OK"
        );
        write(&mut transport, 0x70, 1 | 2 | FEATURES_OK | DRIVER_OK);
        assert!(!write(&mut transport, 0x50, 1), "no such queue");
        assert!(write(&mut transport, 0x50, 0));
        assert_eq!(
            (read(&transport, 0x60), transport.device.0),
            (USED_BUFFER, 1)
        );
        write(&mut transport, 0x64, USED_BUFFER);
        assert_eq!(read(&transport, 0x60), 0);
        write(&mut transport, 0x70, 0);
        assert_eq!(
            (
                read(&transport, 0x44),
                read(&transport, 0x70),
                transport.device.0
            ),
            (0, 0, 0)
        );
    }
}
