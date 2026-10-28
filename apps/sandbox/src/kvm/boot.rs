//! The Linux x86 boot protocol's 64-bit entry (Documentation/arch/x86/boot.rst): places a bzImage,
//! initramfs, command line, zero page, GDT and identity page tables in guest memory.
use std::io;

pub const ZERO_PAGE: u64 = 0x7000;
pub const STACK: u64 = 0x8ff0;
pub const GDT: u64 = 0x500;
pub const PML4: u64 = 0x9000;
const CMDLINE: u64 = 0x2_0000;
const KERNEL: u64 = 0x10_0000;
const EBDA: u64 = 0x9_fc00;

/// Loads the guest and returns the kernel's 64-bit entry point.
pub fn load(memory: &mut [u8], kernel: &[u8], initramfs: &[u8], cmdline: &str) -> io::Result<u64> {
    let header = Header::parse(kernel)?;
    let payload = &kernel[header.payload_offset..];
    put(memory, KERNEL, payload)?;
    // The decompressor works in [pref_address, pref_address + init_size); keep the initramfs above it.
    let reserved = (KERNEL + payload.len() as u64).max(header.pref_address + header.init_size);
    let top = (memory.len() as u64).min(header.initrd_addr_max + 1);
    let initrd = top
        .checked_sub(initramfs.len() as u64)
        .map(|address| address & !0xfff)
        .filter(|&address| address >= reserved)
        .ok_or_else(|| invalid("guest memory too small for the kernel and initramfs"))?;
    put(memory, initrd, initramfs)?;
    if cmdline.len() >= header.cmdline_size {
        return Err(invalid("kernel command line too long"));
    }
    put(memory, CMDLINE, cmdline.as_bytes())?;
    put(memory, CMDLINE + cmdline.len() as u64, &[0])?;
    zero_page(memory, kernel, initrd, initramfs.len() as u32)?;
    page_tables(memory)?;
    // Null, null, 64-bit code at selector 0x10, data at 0x18: the selectors the kernel expects.
    let gdt = [0u64, 0, 0x00af_9b00_0000_ffff, 0x00cf_9300_0000_ffff];
    put(memory, GDT, &gdt.map(u64::to_le_bytes).concat())?;
    Ok(KERNEL + 0x200)
}

struct Header {
    payload_offset: usize,
    pref_address: u64,
    init_size: u64,
    initrd_addr_max: u64,
    cmdline_size: usize,
}

impl Header {
    fn parse(kernel: &[u8]) -> io::Result<Self> {
        if kernel.len() < 0x268 || &kernel[0x202..0x206] != b"HdrS" {
            return Err(invalid("guest kernel is not a bzImage"));
        }
        // 2.12 added xloadflags, whose bit 0 advertises the 64-bit entry point.
        if u16_at(kernel, 0x206) < 0x020c || u16_at(kernel, 0x236) & 1 == 0 {
            return Err(invalid("guest kernel has no 64-bit boot entry"));
        }
        let setup_sects = match kernel[0x1f1] {
            0 => 4,
            sectors => sectors as usize,
        };
        let payload_offset = (setup_sects + 1) * 512;
        if payload_offset >= kernel.len() {
            return Err(invalid("guest kernel is truncated"));
        }
        Ok(Self {
            payload_offset,
            pref_address: u64::from_le_bytes(kernel[0x258..0x260].try_into().unwrap()),
            init_size: u32_at(kernel, 0x260).into(),
            initrd_addr_max: u32_at(kernel, 0x22c).into(),
            cmdline_size: u32_at(kernel, 0x238) as usize,
        })
    }
}

fn zero_page(memory: &mut [u8], kernel: &[u8], initrd: u64, initrd_size: u32) -> io::Result<()> {
    let mut page = [0u8; 4096];
    // The setup header runs from 0x1f1 to the end its own length byte at 0x201 declares.
    let end = 0x202 + kernel[0x201] as usize;
    page[0x1f1..end].copy_from_slice(&kernel[0x1f1..end]);
    page[0x210] = 0xff; // type_of_loader: undefined
    page[0x218..0x21c].copy_from_slice(&(initrd as u32).to_le_bytes());
    page[0x21c..0x220].copy_from_slice(&initrd_size.to_le_bytes());
    page[0x228..0x22c].copy_from_slice(&(CMDLINE as u32).to_le_bytes());
    let ram = [(0, EBDA), (KERNEL, memory.len() as u64 - KERNEL)];
    page[0x1e8] = ram.len() as u8;
    for (index, (address, size)) in ram.into_iter().enumerate() {
        let entry = 0x2d0 + index * 20;
        page[entry..entry + 8].copy_from_slice(&address.to_le_bytes());
        page[entry + 8..entry + 16].copy_from_slice(&size.to_le_bytes());
        page[entry + 16..entry + 20].copy_from_slice(&1u32.to_le_bytes()); // E820_TYPE_RAM
    }
    put(memory, ZERO_PAGE, &page)
}

/// Identity-maps the first 1 GiB with 2 MiB pages; the kernel builds its own tables early.
fn page_tables(memory: &mut [u8]) -> io::Result<()> {
    let (pdpt, pd) = (PML4 + 0x1000, PML4 + 0x2000);
    put(memory, PML4, &(pdpt | 0x3).to_le_bytes())?;
    put(memory, pdpt, &(pd | 0x3).to_le_bytes())?;
    let entries: Vec<u8> = (0..512u64)
        .flat_map(|index| ((index << 21) | 0x83).to_le_bytes())
        .collect();
    put(memory, pd, &entries)
}

fn put(memory: &mut [u8], address: u64, bytes: &[u8]) -> io::Result<()> {
    usize::try_from(address)
        .ok()
        .and_then(|start| memory.get_mut(start..start.checked_add(bytes.len())?))
        .ok_or_else(|| invalid("guest image does not fit in guest memory"))?
        .copy_from_slice(bytes);
    Ok(())
}

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes(bytes[offset..offset + 4].try_into().unwrap())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bzimage() -> Vec<u8> {
        let mut kernel = vec![0u8; 5 * 512 + 16];
        kernel[0x1f1] = 4;
        kernel[0x201] = 0x66; // header ends at 0x268
        kernel[0x202..0x206].copy_from_slice(b"HdrS");
        kernel[0x206..0x208].copy_from_slice(&0x020fu16.to_le_bytes());
        kernel[0x22c..0x230].copy_from_slice(&0x7fff_ffffu32.to_le_bytes());
        kernel[0x236] = 1;
        kernel[0x238..0x23c].copy_from_slice(&2048u32.to_le_bytes());
        kernel[0x258..0x260].copy_from_slice(&0x100_0000u64.to_le_bytes());
        kernel[0x260..0x264].copy_from_slice(&0x40_0000u32.to_le_bytes());
        kernel[5 * 512] = 0xe9;
        kernel
    }

    #[test]
    fn places_the_kernel_initramfs_and_boot_parameters() {
        let mut memory = vec![0u8; 64 << 20];
        let entry = load(&mut memory, &bzimage(), b"07070100", "console=ttyS0").unwrap();
        assert_eq!(entry, 0x10_0200);
        assert_eq!(memory[0x10_0000], 0xe9);
        let page = &memory[0x7000..0x8000];
        let initrd = u32_at(page, 0x218) as usize;
        assert_eq!(initrd, (64 << 20) - 0x1000);
        assert_eq!(&memory[initrd..initrd + 8], b"07070100");
        assert_eq!(u32_at(page, 0x21c), 8);
        assert_eq!(page[0x210], 0xff);
        assert_eq!(&page[0x202..0x206], b"HdrS");
        assert_eq!(page[0x1e8], 2);
        assert_eq!(&memory[0x2_0000..0x2_000e], b"console=ttyS0\0");
        assert_eq!(u64_at(&memory, PML4), 0xa003);
        assert_eq!(u64_at(&memory, 0xb000 + 8), 0x20_0083);
    }

    #[test]
    fn rejects_images_that_cannot_boot() {
        let mut memory = vec![0u8; 64 << 20];
        let mut old = bzimage();
        old[0x236] = 0;
        assert!(load(&mut memory, &old, b"", "").is_err());
        assert!(load(&mut memory, b"not a kernel", b"", "").is_err());
        // The initramfs may not overlap the decompressor's 16-20 MiB working area.
        let mut small = vec![0u8; 20 << 20];
        assert!(load(&mut small, &bzimage(), &[0; 0x1000], "").is_err());
    }

    fn u64_at(bytes: &[u8], offset: u64) -> u64 {
        let offset = offset as usize;
        u64::from_le_bytes(bytes[offset..offset + 8].try_into().unwrap())
    }
}
