//! The KVM ABI subset this VMM uses, from `linux/kvm.h` for x86_64.
use nix::{
    ioctl_read, ioctl_readwrite_bad, ioctl_write_int_bad, ioctl_write_ptr, ioctl_write_ptr_bad,
    request_code_none, request_code_readwrite, request_code_write, sys::ioctl::ioctl_num_type,
};

const KVMIO: u8 = 0xAE;
pub const API_VERSION: i32 = 12;
pub const EXIT_IO: u32 = 2;
pub const EXIT_HLT: u32 = 5;
pub const EXIT_MMIO: u32 = 6;
pub const EXIT_SHUTDOWN: u32 = 8;
pub const EXIT_IO_OUT: u8 = 1;
/// The ioctl takes the address as an unsigned long, which the int-argument macros would sign-extend.
pub const SET_TSS_ADDR: ioctl_num_type = request_code_none!(KVMIO, 0x47);

#[repr(C)]
pub struct MemoryRegion {
    pub slot: u32,
    pub flags: u32,
    pub guest_phys_addr: u64,
    pub memory_size: u64,
    pub userspace_addr: u64,
}

#[repr(C)]
#[derive(Default)]
pub struct PitConfig {
    pub flags: u32,
    pub pad: [u32; 15],
}

#[repr(C)]
pub struct IrqLevel {
    pub irq: u32,
    pub level: u32,
}

#[repr(C)]
#[derive(Default)]
pub struct Regs {
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rsp: u64,
    pub rbp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct Segment {
    pub base: u64,
    pub limit: u32,
    pub selector: u16,
    pub kind: u8,
    pub present: u8,
    pub dpl: u8,
    pub db: u8,
    pub s: u8,
    pub l: u8,
    pub g: u8,
    pub avl: u8,
    pub unusable: u8,
    pub padding: u8,
}

#[repr(C)]
#[derive(Default)]
pub struct Dtable {
    pub base: u64,
    pub limit: u16,
    pub padding: [u16; 3],
}

#[repr(C)]
#[derive(Default)]
pub struct Sregs {
    pub cs: Segment,
    pub ds: Segment,
    pub es: Segment,
    pub fs: Segment,
    pub gs: Segment,
    pub ss: Segment,
    pub tr: Segment,
    pub ldt: Segment,
    pub gdt: Dtable,
    pub idt: Dtable,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub cr8: u64,
    pub efer: u64,
    pub apic_base: u64,
    pub interrupt_bitmap: [u64; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct CpuidEntry {
    pub function: u32,
    pub index: u32,
    pub flags: u32,
    pub eax: u32,
    pub ebx: u32,
    pub ecx: u32,
    pub edx: u32,
    pub padding: [u32; 3],
}

/// `kvm_cpuid2` with its flexible array sized to KVM_MAX_CPUID_ENTRIES.
#[repr(C)]
#[derive(Clone)]
pub struct Cpuid {
    pub nent: u32,
    pub padding: u32,
    pub entries: [CpuidEntry; 256],
}

/// The head of `kvm_run` and its exit union.
#[repr(C)]
pub struct Run {
    pub request_interrupt_window: u8,
    pub immediate_exit: u8,
    pub padding: [u8; 6],
    pub exit_reason: u32,
    pub ready_for_interrupt_injection: u8,
    pub if_flag: u8,
    pub flags: u16,
    pub cr8: u64,
    pub apic_base: u64,
    pub exit: Exit,
}

/// Which member is valid follows from `exit_reason`.
#[repr(C)]
pub union Exit {
    pub io: Io,
    pub mmio: Mmio,
    padding: [u64; 32],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Io {
    pub direction: u8,
    pub size: u8,
    pub port: u16,
    pub count: u32,
    pub data_offset: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Mmio {
    pub phys_addr: u64,
    pub data: [u8; 8],
    pub len: u32,
    pub is_write: u8,
}

// KVM rejects a nonzero argument on several _IO ioctls, so pass an explicit 0 rather than ioctl_none!,
// which leaves the argument register undefined.
ioctl_write_int_bad!(get_api_version, request_code_none!(KVMIO, 0x00));
ioctl_write_int_bad!(create_vm, request_code_none!(KVMIO, 0x01));
ioctl_write_int_bad!(get_vcpu_mmap_size, request_code_none!(KVMIO, 0x04));
ioctl_readwrite_bad!(
    get_supported_cpuid,
    request_code_readwrite!(KVMIO, 0x05, 8),
    Cpuid
);
ioctl_write_int_bad!(create_vcpu, request_code_none!(KVMIO, 0x41));
ioctl_write_ptr!(set_user_memory_region, KVMIO, 0x46, MemoryRegion);
ioctl_write_int_bad!(create_irqchip, request_code_none!(KVMIO, 0x60));
ioctl_write_ptr!(irq_line, KVMIO, 0x61, IrqLevel);
ioctl_write_ptr!(create_pit2, KVMIO, 0x77, PitConfig);
ioctl_write_int_bad!(run, request_code_none!(KVMIO, 0x80));
ioctl_write_ptr!(set_regs, KVMIO, 0x82, Regs);
ioctl_read!(get_sregs, KVMIO, 0x83, Sregs);
ioctl_write_ptr!(set_sregs, KVMIO, 0x84, Sregs);
ioctl_write_ptr_bad!(set_cpuid2, request_code_write!(KVMIO, 0x90, 8), Cpuid);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn structures_match_the_kernel_abi() {
        assert_eq!(size_of::<Regs>(), 144);
        assert_eq!(size_of::<Sregs>(), 312);
        assert_eq!(size_of::<CpuidEntry>(), 40);
        assert_eq!(size_of::<PitConfig>(), 64);
        assert_eq!(std::mem::offset_of!(Run, exit_reason), 8);
        assert_eq!(std::mem::offset_of!(Run, exit), 32);
        assert_eq!(size_of::<Mmio>(), 24);
    }
}
