//! `kvm` backend: one VM with one memory region and one vCPU booting a Linux guest from a read-only
//! root disk, with the agent's guest init as init (ADR 0084). Its devices are a 16550 serial
//! console relayed to stdout and virtio-mmio disks for the root and workspace, a network device on
//! a tap, and vsock for the control channel.
#![allow(unsafe_code, reason = "KVM ioctls and guest memory (ADR 0084)")]
mod boot;
mod memory;
mod network;
mod serial;
mod sys;
mod virtio;
use memory::GuestMemory;
use nix::{
    errno::Errno,
    libc,
    sys::{
        mman::{MapFlags, ProtFlags, mmap},
        signal::{SigSet, Signal},
    },
};
use std::{
    ffi::OsString,
    fs::File,
    io::{self, Read},
    num::NonZeroUsize,
    os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd},
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};
use virtio::{Mmio, Slot, block::Block, net::Net, vsock::Vsock};

// shortcut: fixed guest and workspace sizes, make them configurable once ADR 0084's sizing detail
// is decided.
const MEMORY: usize = 256 << 20;
const WORKSPACE_SIZE: u64 = 8 << 30;
const CMDLINE: &str = "console=ttyS0 earlyprintk=serial reboot=t panic=-1 \
    root=/dev/vda ro rootfstype=ext4 init=/usr/local/bin/cubebot-agent";
const SERIAL: std::ops::RangeInclusive<u16> = 0x3f8..=0x3ff;
const SERIAL_IRQ: u32 = 4;
/// Device windows sit above guest RAM, which the E820 map ends at `MEMORY`.
const MMIO: u64 = 0xd000_0000;
/// Root disk, workspace disk, network, vsock: free ISA interrupts once COM1 has IRQ 4. Linux names
/// the disks vda and vdb in this order.
const IRQS: [u32; 4] = [5, 6, 9, 10];
/// The guest owns its storage paths; it mounts the workspace disk itself.
const GUEST_OWNED: [&str; 2] = ["CUBEBOT_JOURNAL_DIRECTORY", "CUBEBOT_WORKSPACE_DIRECTORY"];
const FILE_LIMIT: u64 = 64 << 10;
/// Well inside the kubelet's refresh of a projected token at 80% of its lifetime.
const REFRESH: std::time::Duration = std::time::Duration::from_secs(60);

pub fn run() -> io::Result<u8> {
    // Blocked before any thread starts, so only the stop thread receives them.
    let mut termination = SigSet::empty();
    termination.add(Signal::SIGTERM);
    termination.add(Signal::SIGINT);
    termination
        .thread_block()
        .map_err(failed("pthread_sigmask"))?;
    let kernel = read_configured("CUBEBOT_GUEST_KERNEL")?;
    let (kvm, vm) = open_vm()?;
    let mut memory = guest_memory(&vm)?;
    let Devices {
        bus,
        net,
        tap,
        vsock,
        environment,
        cmdline,
    } = devices(vm.as_raw_fd())?;
    let entry = boot::load(memory.as_mut_slice(), &kernel, &cmdline)?;
    let memory: &'static GuestMemory = Box::leak(Box::new(memory));
    let vcpu = unsafe {
        OwnedFd::from_raw_fd(
            sys::create_vcpu(vm.as_raw_fd(), 0).map_err(failed("KVM_CREATE_VCPU"))?,
        )
    };
    configure_vcpu(&kvm, &vcpu, entry)?;
    let receivable = net.lock().device.receivable();
    std::thread::spawn(move || {
        let error = virtio::net::receive(tap, &net, &receivable, memory).unwrap_err();
        fail(&format!("guest network: {error}"));
    });
    let refreshed = vsock.clone();
    std::thread::spawn(move || refresh(environment, &refreshed, memory));
    let stopped = vsock.clone();
    std::thread::spawn(move || stop(termination, &stopped, memory));
    run_vcpu(&kvm, &vcpu, vm.as_raw_fd(), &bus, memory)?;
    // A guest that ends without reporting an exit status failed before or while running the agent.
    Ok(vsock.lock().device.exit().unwrap_or_else(|| {
        eprintln!("Guest ended without an agent exit status");
        1
    }))
}

fn open_vm() -> io::Result<(File, OwnedFd)> {
    let kvm = File::options()
        .read(true)
        .write(true)
        .open("/dev/kvm")
        .map_err(|error| io::Error::other(format!("/dev/kvm: {error}")))?;
    if unsafe { sys::get_api_version(kvm.as_raw_fd(), 0) }.map_err(failed("KVM_GET_API_VERSION"))?
        != sys::API_VERSION
    {
        return Err(io::Error::other("unsupported KVM API version"));
    }
    let vm = unsafe {
        OwnedFd::from_raw_fd(sys::create_vm(kvm.as_raw_fd(), 0).map_err(failed("KVM_CREATE_VM"))?)
    };
    Ok((kvm, vm))
}

struct Devices {
    bus: [Arc<dyn Mmio>; 4],
    net: Arc<Slot<Net>>,
    /// The tap's receive side, read by its own thread.
    tap: File,
    vsock: Arc<Slot<Vsock>>,
    /// The environment the guest receives first.
    environment: Vec<(String, String)>,
    /// The kernel command line, which declares the devices and the guest's network.
    cmdline: String,
}

fn devices(vm: RawFd) -> io::Result<Devices> {
    let root = File::open(configured("CUBEBOT_GUEST_ROOT")?)?;
    let workspace = workspace_disk(&configured("CUBEBOT_GUEST_WORKSPACE")?)?;
    let environment = guest_environment(std::env::vars_os())?;
    let encoded = cubebot_protocol::guest::encode(&environment).map_err(io::Error::other)?;
    let network = network::attach()?;
    let irq = |index: usize| Irq {
        vm,
        line: IRQS[index],
    };
    let net = Net::new(network.tap.try_clone()?, network.mac, network.mtu);
    let net = Arc::new(Slot::new(net, irq(2)));
    let vsock = Arc::new(Slot::new(Vsock::new(encoded), irq(3)));
    let bus: [Arc<dyn Mmio>; 4] = [
        Arc::new(Slot::new(Block::new(root, true)?, irq(0))),
        Arc::new(Slot::new(Block::new(workspace, false)?, irq(1))),
        net.clone(),
        vsock.clone(),
    ];
    let devices: String = (0..bus.len())
        .map(|index| {
            format!(
                " virtio_mmio.device=4K@{:#x}:{}",
                mmio_base(index),
                IRQS[index]
            )
        })
        .collect();
    Ok(Devices {
        bus,
        net,
        tap: network.tap,
        vsock,
        environment,
        cmdline: format!("{CMDLINE} {}{devices} -- guest", network.cmdline),
    })
}

/// Runs the vCPU until the guest shuts down, serving its port I/O and MMIO.
fn run_vcpu(
    kvm: &File,
    vcpu: &OwnedFd,
    vm: RawFd,
    bus: &[Arc<dyn Mmio>],
    memory: &GuestMemory,
) -> io::Result<()> {
    let size = unsafe { sys::get_vcpu_mmap_size(kvm.as_raw_fd(), 0) }
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
        unsafe { sys::run(vcpu.as_raw_fd(), 0) }.map_err(failed("KVM_RUN"))?;
        let run = unsafe { run.as_ptr().as_mut().ok_or(Errno::EFAULT)? };
        match run.exit_reason {
            sys::EXIT_IO => port_io(run, &mut serial, vm)?,
            sys::EXIT_MMIO => mmio(run, bus, memory)?,
            // reboot=t and panic=-1 turn a guest reboot, including init exiting, into a triple fault.
            sys::EXIT_HLT | sys::EXIT_SHUTDOWN => return Ok(()),
            reason => return Err(io::Error::other(format!("unexpected vCPU exit {reason}"))),
        }
    }
}

/// The first SIGTERM or SIGINT asks the guest to stop over its control channel, so it unmounts
/// its disks; a second one, or one before the channel is open, ends the VM at once.
fn stop(termination: SigSet, vsock: &Slot<Vsock>, memory: &GuestMemory) {
    let Ok(signal) = termination.wait() else {
        fail("sigwait failed");
    };
    let mut open = false;
    let asked = vsock.run(|vsock, queues| {
        open = vsock.shutdown();
        vsock.deliver(queues, memory)
    });
    let signal = match asked {
        Ok(()) if open => termination.wait().unwrap_or(signal),
        _ => signal,
    };
    std::process::exit(128 + signal as i32)
}

/// Sends the guest every `*_FILE` whose contents changed, such as a projected token the kubelet
/// rotated, since the guest holds only copies.
fn refresh(mut sent: Vec<(String, String)>, vsock: &Slot<Vsock>, memory: &GuestMemory) -> ! {
    loop {
        std::thread::sleep(REFRESH);
        let current = match guest_environment(std::env::vars_os()) {
            Ok(current) => current,
            Err(error) => {
                eprintln!("Guest file refresh failed: {error}");
                continue;
            }
        };
        let changed: Vec<_> = current
            .iter()
            .filter(|entry| entry.0.ends_with("_FILE") && !sent.contains(entry))
            .cloned()
            .collect();
        if changed.is_empty() {
            continue;
        }
        let encode = cubebot_protocol::guest::encode;
        let (Ok(environment), Ok(changed)) = (encode(&current), encode(&changed)) else {
            fail("refreshed guest environment cannot be encoded");
        };
        let delivered = vsock.run(|vsock, queues| {
            vsock.refresh(environment, &changed);
            vsock.deliver(queues, memory)
        });
        if let Err(error) = delivered {
            fail(&format!("guest control channel: {error}"));
        }
        sent = current;
    }
}

/// A device thread cannot return its error to `run`; it ends the sandbox instead.
fn fail(message: &str) -> ! {
    eprintln!("Sandbox failed: {message}");
    std::process::exit(1)
}

fn failed(operation: &'static str) -> impl Fn(Errno) -> io::Error {
    move |error| io::Error::other(format!("{operation}: {error}"))
}

fn configured(name: &str) -> io::Result<PathBuf> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| io::Error::other(format!("{name} is not set")))
}

fn read_configured(name: &str) -> io::Result<Vec<u8>> {
    let path = configured(name)?;
    std::fs::read(&path)
        .map_err(|error| io::Error::other(format!("{name} {}: {error}", path.display())))
}

/// Opens the workspace disk, creating and formatting it only when it does not exist, so an
/// existing disk is never reformatted. The root directory belongs to root, not the tool user.
fn workspace_disk(path: &Path) -> io::Result<File> {
    if !path.try_exists()? {
        let partial = path.with_extension("partial");
        File::create(&partial)?.set_len(WORKSPACE_SIZE)?;
        let status = Command::new("mkfs.ext4")
            .args(["-q", "-F", "-L", "workspace", "-E", "root_owner=0:0"])
            .arg(&partial)
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!(
                "mkfs.ext4 {}: {status}",
                partial.display()
            )));
        }
        std::fs::rename(&partial, path)?;
    }
    File::options().read(true).write(true).open(path)
}

/// The agent's `CUBEBOT_*` environment as the guest receives it (`cubebot_protocol::guest`): no
/// sandbox or storage settings, and each `*_FILE` variable replaced by the file's contents.
fn guest_environment(
    variables: impl IntoIterator<Item = (OsString, OsString)>,
) -> io::Result<Vec<(String, String)>> {
    let mut environment = Vec::new();
    for (name, value) in variables {
        let Some(name) = name.to_str().filter(|name| name.starts_with("CUBEBOT_")) else {
            continue;
        };
        if name.starts_with("CUBEBOT_GUEST_") || GUEST_OWNED.contains(&name) {
            continue;
        }
        let value = value
            .into_string()
            .map_err(|_| io::Error::other(format!("{name} is not UTF-8")))?;
        let value = match name.ends_with("_FILE") {
            true => read_bounded(Path::new(&value))
                .map_err(|error| io::Error::other(format!("{name} {value}: {error}")))?,
            false => value,
        };
        environment.push((name.to_owned(), value));
    }
    Ok(environment)
}

fn read_bounded(path: &Path) -> io::Result<String> {
    let mut contents = String::new();
    File::open(path)?
        .take(FILE_LIMIT + 1)
        .read_to_string(&mut contents)?;
    if contents.len() as u64 > FILE_LIMIT {
        return Err(io::Error::other("file exceeds 64 KiB"));
    }
    Ok(contents)
}

fn mmio_base(index: usize) -> u64 {
    MMIO + index as u64 * virtio::WINDOW
}

/// Maps the single guest memory region at guest physical address 0, with the in-kernel PIC, IOAPIC
/// and PIT the guest's timer and device interrupts need.
fn guest_memory(vm: &OwnedFd) -> io::Result<GuestMemory> {
    let fd = vm.as_raw_fd();
    let memory = GuestMemory::map(MEMORY)?;
    let region = sys::MemoryRegion {
        slot: 0,
        flags: 0,
        guest_phys_addr: 0,
        memory_size: memory.size() as u64,
        userspace_addr: memory.host_address(),
    };
    unsafe {
        sys::set_user_memory_region(fd, &region).map_err(failed("KVM_SET_USER_MEMORY_REGION"))?;
        // Intel VMX needs three pages for the real-mode TSS; this is the address every VMM uses.
        Errno::result(libc::ioctl(fd, sys::SET_TSS_ADDR, 0xfffb_d000_u64))
            .map_err(failed("KVM_SET_TSS_ADDR"))?;
        sys::create_irqchip(fd, 0).map_err(failed("KVM_CREATE_IRQCHIP"))?;
        sys::create_pit2(fd, &sys::PitConfig::default()).map_err(failed("KVM_CREATE_PIT2"))?;
    }
    Ok(memory)
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
        as_vcpu_zero(&mut cpuid);
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

/// The supported CPUID carries the host CPU's APIC ID; the guest's only CPU is APIC 0.
fn as_vcpu_zero(cpuid: &mut sys::Cpuid) {
    for entry in cpuid.entries.iter_mut().take(cpuid.nent as usize) {
        match entry.function {
            0x1 => entry.ebx &= 0x00ff_ffff,
            0xb | 0x1f => entry.edx = 0,
            _ => (),
        }
    }
}

/// Serves the 16550 at COM1; other ports read as zero and ignore writes, as on an empty bus.
fn port_io(run: &mut sys::Run, serial: &mut serial::Serial, vm: RawFd) -> io::Result<()> {
    let sys::Io {
        direction,
        size,
        port,
        count,
        data_offset,
    } = unsafe { run.exit.io };
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
                    Irq {
                        vm,
                        line: SERIAL_IRQ,
                    }
                    .raise()?;
                }
            }
        }
        (true, false) => data.fill_with(|| serial.read(port - SERIAL.start())),
        (false, true) => (),
        (false, false) => data.fill(0),
    }
    Ok(())
}

/// Serves a virtio-mmio window; addresses outside every window read as zero and ignore writes.
fn mmio(run: &mut sys::Run, bus: &[Arc<dyn Mmio>], memory: &GuestMemory) -> io::Result<()> {
    let exit = unsafe { &mut run.exit.mmio };
    let data = &mut exit.data[..(exit.len as usize).min(8)];
    let index = exit
        .phys_addr
        .checked_sub(MMIO)
        .map(|offset| offset / virtio::WINDOW);
    let Some(device) = index.and_then(|index| bus.get(index as usize)) else {
        data.fill(0);
        return Ok(());
    };
    let offset = (exit.phys_addr - MMIO) % virtio::WINDOW;
    if exit.is_write == 0 {
        device.read(offset, data);
        return Ok(());
    }
    device.write(offset, data, memory)
}

/// An edge on one of the in-kernel PIC's lines.
#[derive(Clone, Copy)]
pub struct Irq {
    vm: RawFd,
    line: u32,
}

impl Irq {
    pub fn raise(&self) -> io::Result<()> {
        for level in [1, 0] {
            let level = sys::IrqLevel {
                irq: self.line,
                level,
            };
            unsafe { sys::irq_line(self.vm, &level) }.map_err(failed("KVM_IRQ_LINE"))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpuid_reports_apic_zero_and_keeps_other_leaves() {
        let mut cpuid = sys::Cpuid {
            nent: 3,
            padding: 0,
            entries: [sys::CpuidEntry::default(); 256],
        };
        for (entry, function) in cpuid.entries.iter_mut().zip([0x1, 0xb, 0x7]) {
            *entry = sys::CpuidEntry {
                function,
                ebx: 0x0201_0800,
                edx: 2,
                ..Default::default()
            };
        }
        as_vcpu_zero(&mut cpuid);
        assert_eq!(cpuid.entries[0].ebx, 0x0001_0800);
        assert_eq!(cpuid.entries[1].edx, 0);
        assert_eq!(
            (cpuid.entries[2].ebx, cpuid.entries[2].edx),
            (0x0201_0800, 2)
        );
    }

    #[test]
    fn the_guest_receives_agent_settings_and_file_contents() {
        let token = std::env::temp_dir().join(format!("cubebot-token-{}", std::process::id()));
        std::fs::write(&token, "secret").unwrap();
        let variables = [
            ("CUBEBOT_ORCHESTRATOR_ORIGIN", "https://10.0.0.1/"),
            ("CUBEBOT_BOOTSTRAP_TOKEN_FILE", token.to_str().unwrap()),
            ("CUBEBOT_GUEST_KERNEL", "/bzImage"),
            ("CUBEBOT_JOURNAL_DIRECTORY", "/var/lib/cubebot/runtime"),
            ("PATH", "/bin"),
        ]
        .map(|(name, value)| (name.into(), value.into()));
        let environment = guest_environment(variables).unwrap();
        std::fs::remove_file(&token).unwrap();
        let expected = [
            ("CUBEBOT_ORCHESTRATOR_ORIGIN", "https://10.0.0.1/"),
            ("CUBEBOT_BOOTSTRAP_TOKEN_FILE", "secret"),
        ]
        .map(|(name, value)| (name.to_owned(), value.to_owned()));
        assert_eq!(environment, expected);
        let missing = [("CUBEBOT_CONTROL_CA_FILE".into(), "/missing".into())];
        assert!(guest_environment(missing).is_err());
    }
}
