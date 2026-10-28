//! `kvm` backend: one VM with one memory region and one vCPU booting a Linux guest whose init is the
//! agent. The guest serial console is relayed to stdout.
#![allow(unsafe_code, reason = "KVM ioctls and guest memory (ADR 0084)")]
mod boot;
mod serial;
mod sys;
use nix::{
    errno::Errno,
    libc,
    sys::{
        mman::{MapFlags, ProtFlags, mmap, mmap_anonymous},
        signal::{SaFlags, SigAction, SigHandler, SigSet, Signal, sigaction},
    },
};
use std::{
    fs::File,
    io,
    num::NonZeroUsize,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
};

// shortcut: fixed guest size, make it configurable once ADR 0084's sizing detail is decided.
const MEMORY: usize = 256 << 20;
const CMDLINE: &str = "console=ttyS0 earlyprintk=serial reboot=t panic=-1";
const SERIAL: std::ops::RangeInclusive<u16> = 0x3f8..=0x3ff;
const SERIAL_IRQ: u32 = 4;

pub fn run() -> io::Result<u8> {
    exit_on_termination()?;
    let kernel = read_configured("CUBEBOT_GUEST_KERNEL")?;
    let initramfs = read_configured("CUBEBOT_GUEST_INITRAMFS")?;
    let kvm = File::options()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .map_err(|error| io::Error::other(format!("/dev/kvm: {error}")))?;
    if unsafe { sys::get_api_version(kvm.as_raw_fd()) }.map_err(failed("KVM_GET_API_VERSION"))?
        != sys::API_VERSION
    {
        return Err(io::Error::other("unsupported KVM API version"));
    }
    let vm = unsafe {
        OwnedFd::from_raw_fd(sys::create_vm(kvm.as_raw_fd(), 0).map_err(failed("KVM_CREATE_VM"))?)
    };
    let memory = guest_memory(&vm)?;
    let entry = boot::load(memory, &kernel, &initramfs, CMDLINE)?;
    let vcpu = unsafe {
        OwnedFd::from_raw_fd(
            sys::create_vcpu(vm.as_raw_fd(), 0).map_err(failed("KVM_CREATE_VCPU"))?,
        )
    };
    configure_vcpu(&kvm, &vcpu, entry)?;
    run_vcpu(&kvm, &vcpu, vm.as_raw_fd())
}

/// Runs the vCPU until the guest shuts down, serving its port I/O.
fn run_vcpu(kvm: &File, vcpu: &OwnedFd, vm: RawFd) -> io::Result<u8> {
    let size = unsafe { sys::get_vcpu_mmap_size(kvm.as_raw_fd()) }
        .map_err(failed("KVM_GET_VCPU_MMAP_SIZE"))?;
    let run = unsafe {
        mmap(
            None,
            NonZeroUsize::new(size as usize).ok_or(Errno::EINVAL)?,
            ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
            MapFlags::MAP_SHARED,
            vcpu,
            0,
        )
    }
    .map_err(failed("mmap kvm_run"))?
    .cast::<sys::Run>();
    let mut serial = serial::Serial::default();
    loop {
        unsafe { sys::run(vcpu.as_raw_fd()) }.map_err(failed("KVM_RUN"))?;
        let run = unsafe { run.as_ptr().as_mut().ok_or(Errno::EFAULT)? };
        match run.exit_reason {
            sys::EXIT_IO => port_io(run, &mut serial, vm)?,
            // reboot=t and panic=-1 turn a guest reboot, including init exiting, into a triple fault.
            sys::EXIT_HLT | sys::EXIT_SHUTDOWN => return Ok(0),
            reason => return Err(io::Error::other(format!("unexpected vCPU exit {reason}"))),
        }
    }
}

/// As container PID 1 the default SIGTERM disposition is ignored; stopping the sandbox ends the VM.
fn exit_on_termination() -> io::Result<()> {
    extern "C" fn terminate(signal: libc::c_int) {
        unsafe { libc::_exit(128 + signal) }
    }
    let action = SigAction::new(
        SigHandler::Handler(terminate),
        SaFlags::empty(),
        SigSet::empty(),
    );
    for signal in [Signal::SIGTERM, Signal::SIGINT] {
        unsafe { sigaction(signal, &action) }.map_err(failed("sigaction"))?;
    }
    Ok(())
}

fn failed(operation: &'static str) -> impl Fn(Errno) -> io::Error {
    move |error| io::Error::other(format!("{operation}: {error}"))
}

fn read_configured(name: &str) -> io::Result<Vec<u8>> {
    let path =
        std::env::var_os(name).ok_or_else(|| io::Error::other(format!("{name} is not set")))?;
    std::fs::read(&path)
        .map_err(|error| io::Error::other(format!("{name} {}: {error}", path.display())))
}

/// Maps the single guest memory region at guest physical address 0, with the in-kernel PIC, IOAPIC
/// and PIT the guest's timer and serial interrupts need.
fn guest_memory(vm: &OwnedFd) -> io::Result<&'static mut [u8]> {
    let fd = vm.as_raw_fd();
    let host = unsafe {
        mmap_anonymous(
            None,
            NonZeroUsize::new(MEMORY).ok_or(Errno::EINVAL)?,
            ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
            MapFlags::MAP_PRIVATE | MapFlags::MAP_NORESERVE,
        )
    }
    .map_err(failed("mmap guest memory"))?;
    let region = sys::MemoryRegion {
        slot: 0,
        flags: 0,
        guest_phys_addr: 0,
        memory_size: MEMORY as u64,
        userspace_addr: host.as_ptr() as u64,
    };
    unsafe {
        sys::set_user_memory_region(fd, &region).map_err(failed("KVM_SET_USER_MEMORY_REGION"))?;
        // Intel VMX needs three pages for the real-mode TSS; this is the address every VMM uses.
        Errno::result(libc::ioctl(fd, sys::SET_TSS_ADDR, 0xfffb_d000_u64))
            .map_err(failed("KVM_SET_TSS_ADDR"))?;
        sys::create_irqchip(fd).map_err(failed("KVM_CREATE_IRQCHIP"))?;
        sys::create_pit2(fd, &sys::PitConfig::default()).map_err(failed("KVM_CREATE_PIT2"))?;
        // The mapping lives until the process exits, and the host touches it only before the vCPU runs.
        Ok(std::slice::from_raw_parts_mut(host.as_ptr().cast(), MEMORY))
    }
}

/// Starts the vCPU in 64-bit long mode at the kernel entry with the host's supported CPUID.
fn configure_vcpu(kvm: &File, vcpu: &OwnedFd, entry: u64) -> io::Result<()> {
    let fd = vcpu.as_raw_fd();
    let mut cpuid = sys::Cpuid {
        nent: 256,
        padding: 0,
        entries: [sys::CpuidEntry::default(); 256],
    };
    let mut sregs = sys::Sregs::default();
    unsafe {
        sys::get_supported_cpuid(kvm.as_raw_fd(), &mut cpuid)
            .map_err(failed("KVM_GET_SUPPORTED_CPUID"))?;
        sys::set_cpuid2(fd, &cpuid).map_err(failed("KVM_SET_CPUID2"))?;
        sys::get_sregs(fd, &mut sregs).map_err(failed("KVM_GET_SREGS"))?;
    }
    let code = sys::Segment {
        limit: 0xffff_ffff,
        selector: 0x10,
        kind: 0xb,
        present: 1,
        s: 1,
        l: 1,
        g: 1,
        ..Default::default()
    };
    let data = sys::Segment {
        selector: 0x18,
        kind: 0x3,
        db: 1,
        l: 0,
        ..code
    };
    sregs.cs = code;
    (sregs.ds, sregs.es, sregs.fs, sregs.gs, sregs.ss) = (data, data, data, data, data);
    sregs.gdt.base = boot::GDT;
    sregs.gdt.limit = 31;
    sregs.cr3 = boot::PML4;
    sregs.cr4 |= 0x20; // PAE
    sregs.cr0 |= 0x8000_0001; // PG | PE
    sregs.efer |= 0x500; // LMA | LME
    let regs = sys::Regs {
        rip: entry,
        rsi: boot::ZERO_PAGE,
        rsp: boot::STACK,
        rbp: boot::STACK,
        rflags: 0x2,
        ..Default::default()
    };
    unsafe {
        sys::set_sregs(fd, &sregs).map_err(failed("KVM_SET_SREGS"))?;
        sys::set_regs(fd, &regs).map_err(failed("KVM_SET_REGS"))?;
    }
    Ok(())
}

/// Serves the 16550 at COM1; other ports read as zero and ignore writes, as on an empty bus.
fn port_io(run: &mut sys::Run, serial: &mut serial::Serial, vm: RawFd) -> io::Result<()> {
    let sys::Io {
        direction,
        size,
        port,
        count,
        data_offset,
    } = run.io;
    let data = unsafe {
        std::slice::from_raw_parts_mut(
            (run as *mut sys::Run)
                .cast::<u8>()
                .add(data_offset as usize),
            size as usize * count as usize,
        )
    };
    match (SERIAL.contains(&port), direction == sys::EXIT_IO_OUT) {
        (true, true) => {
            let mut out = io::stdout().lock();
            for &byte in data.iter() {
                if serial.write(port - SERIAL.start(), byte, &mut out)? {
                    pulse(vm, SERIAL_IRQ)?;
                }
            }
        }
        (true, false) => data.fill_with(|| serial.read(port - SERIAL.start())),
        (false, true) => (),
        (false, false) => data.fill(0),
    }
    Ok(())
}

fn pulse(vm: RawFd, irq: u32) -> io::Result<()> {
    for level in [1, 0] {
        unsafe { sys::irq_line(vm, &sys::IrqLevel { irq, level }) }
            .map_err(failed("KVM_IRQ_LINE"))?;
    }
    Ok(())
}
