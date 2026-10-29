//! Guest physical memory, shared by the vCPU and device threads. Devices copy through it and never
//! hold references into it, because the guest may change any byte at any time.
use nix::{
    errno::Errno,
    sys::mman::{MapFlags, ProtFlags, mmap_anonymous},
};
use std::{
    io,
    num::NonZeroUsize,
    ptr::NonNull,
    sync::atomic::{AtomicU16, Ordering},
};

pub struct GuestMemory {
    base: NonNull<u8>,
    size: usize,
}

// The mapping is never unmapped, and every access goes through raw copies or atomics.
unsafe impl Send for GuestMemory {}
unsafe impl Sync for GuestMemory {}

impl GuestMemory {
    pub fn map(size: usize) -> io::Result<Self> {
        let base = unsafe {
            mmap_anonymous(
                None,
                NonZeroUsize::new(size).ok_or(Errno::EINVAL)?,
                ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
                MapFlags::MAP_PRIVATE | MapFlags::MAP_NORESERVE,
            )
        }
        .map_err(|error| io::Error::other(format!("mmap guest memory: {error}")))?;
        Ok(Self {
            base: base.cast(),
            size,
        })
    }

    pub fn host_address(&self) -> u64 {
        self.base.as_ptr() as u64
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// For loading the guest image; exclusive access proves no vCPU or device shares it yet.
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.base.as_ptr(), self.size) }
    }

    pub fn read(&self, address: u64, buffer: &mut [u8]) -> io::Result<()> {
        let start = self.offset(address, buffer.len())?;
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.base.as_ptr().add(start),
                buffer.as_mut_ptr(),
                buffer.len(),
            );
        }
        Ok(())
    }

    pub fn write(&self, address: u64, bytes: &[u8]) -> io::Result<()> {
        let start = self.offset(address, bytes.len())?;
        unsafe {
            std::ptr::copy_nonoverlapping(
                bytes.as_ptr(),
                self.base.as_ptr().add(start),
                bytes.len(),
            );
        }
        Ok(())
    }

    /// Ring indices are read and published atomically so they order the ring entries around them.
    pub fn load_u16(&self, address: u64) -> io::Result<u16> {
        Ok(self.atomic_u16(address)?.load(Ordering::Acquire))
    }

    pub fn store_u16(&self, address: u64, value: u16) -> io::Result<()> {
        self.atomic_u16(address)?.store(value, Ordering::Release);
        Ok(())
    }

    fn atomic_u16(&self, address: u64) -> io::Result<&AtomicU16> {
        if !address.is_multiple_of(2) {
            return Err(invalid("misaligned virtqueue index"));
        }
        let start = self.offset(address, 2)?;
        Ok(unsafe { AtomicU16::from_ptr(self.base.as_ptr().add(start).cast()) })
    }

    fn offset(&self, address: u64, len: usize) -> io::Result<usize> {
        usize::try_from(address)
            .ok()
            .filter(|&start| start.checked_add(len).is_some_and(|end| end <= self.size))
            .ok_or_else(|| invalid("guest address outside guest memory"))
    }
}

pub fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copies_stay_inside_guest_memory() {
        let memory = GuestMemory::map(4096).unwrap();
        memory.write(4090, b"abcdef").unwrap();
        let mut bytes = [0; 6];
        memory.read(4090, &mut bytes).unwrap();
        assert_eq!(&bytes, b"abcdef");
        assert!(memory.write(4091, b"abcdef").is_err());
        assert!(memory.read(u64::MAX, &mut bytes).is_err());
        memory.store_u16(8, 0x1234).unwrap();
        assert_eq!(memory.load_u16(8).unwrap(), 0x1234);
        assert!(memory.load_u16(9).is_err());
        assert!(memory.load_u16(4096).is_err());
    }
}
