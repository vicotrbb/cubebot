//! The sandbox: one VM with one memory region and a thread per vCPU booting a Linux guest from a
//! read-only root disk, with the agent's guest init as init (ADR 0084). Its devices are a 16550 serial
//! console relayed to stdout and virtio-mmio disks for the root and workspace, a network device on
//! a tap, vsock for the control channel, and the desktop's screen, keyboard and tablet (ADR 0085).
#![allow(unsafe_code, reason = "KVM ioctls and guest memory (ADR 0084)")]
mod boot;
mod desktop;
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
    sync::{Arc, Mutex, mpsc},
};
use virtio::{Mmio, Slot, block::Block, gpu::Gpu, input::Input, net::Net, vsock::Vsock};

// shortcut: fixed workspace size, make it configurable when an agent needs more.
const WORKSPACE_SIZE: u64 = 8 << 30;
/// CUBEBOT_GUEST_VCPUS and CUBEBOT_GUEST_MEMORY_MIB (ADR 0085). 32 is the guest kernel's NR_CPUS;
/// shortcut: a 64-vCPU guest hung in reboot on a 16-core host, find why before raising it.
const VCPUS: (u32, std::ops::RangeInclusive<u32>) = (2, 1..=32);
const MEMORY_MIB: (u32, std::ops::RangeInclusive<u32>) = (2048, 256..=3072);
const CMDLINE: &str = "console=ttyS0 earlyprintk=serial reboot=t panic=-1 \
    root=/dev/vda ro rootfstype=ext4 init=/usr/local/bin/cubebot-agent";
const SERIAL: std::ops::RangeInclusive<u16> = 0x3f8..=0x3ff;
const SERIAL_IRQ: u32 = 4;
/// Device windows sit above guest RAM, which the E820 map ends at the guest memory size.
const MMIO: u64 = 0xd000_0000;
const _: () = assert!((*MEMORY_MIB.1.end() as u64) << 20 <= MMIO);
/// Root disk, workspace disk, network, vsock, screen, keyboard, tablet: free ISA interrupts once
/// COM1 has IRQ 4. Linux names the disks vda and vdb in this order.
const IRQS: [u32; 7] = [5, 6, 9, 10, 11, 12, 7];
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
    let vcpus = sized("CUBEBOT_GUEST_VCPUS", VCPUS)? as u8;
    let memory_size = (sized("CUBEBOT_GUEST_MEMORY_MIB", MEMORY_MIB)? as usize) << 20;
    let (kvm, vm) = open_vm()?;
    let mut memory = guest_memory(&vm, memory_size)?;
    let Devices {
        bus,
        net,
        tap,
        vsock,
        gpu,
        keyboard,
        tablet,
        desktop,
        environment,
        cmdline,
    } = devices(vm.as_raw_fd())?;
    let entry = boot::load(memory.as_mut_slice(), &kernel, &cmdline, vcpus)?;
    let memory: &'static GuestMemory = Box::leak(Box::new(memory));
    let vcpus = create_vcpus(&kvm, vm.as_raw_fd(), vcpus, entry)?;
    let size = unsafe { sys::get_vcpu_mmap_size(kvm.as_raw_fd(), 0) }
        .map_err(failed("KVM_GET_VCPU_MMAP_SIZE"))? as usize;
    let receivable = net.lock().device.receivable();
    std::thread::spawn(move || {
        let error = virtio::net::receive(tap, &net, &receivable, memory).unwrap_err();
        fail(&format!("guest network: {error}"));
    });
    let refreshed = vsock.clone();
    std::thread::spawn(move || refresh(environment, &refreshed, memory));
    let stopped = vsock.clone();
    std::thread::spawn(move || stop(termination, &stopped, memory));
    let arbiter = vsock.clone();
    std::thread::spawn(move || {
        desktop::Arbiter::new(&gpu, &keyboard, &tablet, memory).serve(desktop, &arbiter);
    });
    // The guest ends when any vCPU stops, such as the one that triple-faults on reboot.
    let (ended, end) = mpsc::channel();
    let serial = Arc::new(Mutex::new(serial::Serial::default()));
    let vm_fd = vm.as_raw_fd();
    for vcpu in vcpus {
        let (ended, bus, serial) = (ended.clone(), bus.clone(), serial.clone());
        std::thread::spawn(move || {
            let _ = ended.send(run_vcpu(&vcpu, size, vm_fd, &bus, &serial, memory));
        });
    }
    end.recv().map_err(io::Error::other)??;
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
    bus: [Arc<dyn Mmio>; 7],
    net: Arc<Slot<Net>>,
    /// The tap's receive side, read by its own thread.
    tap: File,
    vsock: Arc<Slot<Vsock>>,
    gpu: Arc<Slot<Gpu>>,
    keyboard: Arc<Slot<Input>>,
    tablet: Arc<Slot<Input>>,
    /// Requests for the screen arbiter, from the guest's desktop streams and the host channel.
    desktop: mpsc::Receiver<desktop::Request>,
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
    let (requests, desktop) = mpsc::channel();
    let socket = std::env::var_os("CUBEBOT_GUEST_DESKTOP_SOCKET")
        .map_or_else(|| PathBuf::from(desktop::SOCKET), PathBuf::from);
    desktop::listen(&socket, requests.clone()).map_err(|error| {
        io::Error::other(format!("desktop socket {}: {error}", socket.display()))
    })?;
    let vsock = Arc::new(Slot::new(Vsock::new(encoded, requests), irq(3)));
    let gpu = Arc::new(Slot::new(Gpu::default(), irq(4)));
    let keyboard = Arc::new(Slot::new(Input::keyboard(), irq(5)));
    let tablet = Input::tablet(virtio::gpu::WIDTH, virtio::gpu::HEIGHT);
    let tablet = Arc::new(Slot::new(tablet, irq(6)));
    let bus: [Arc<dyn Mmio>; 7] = [
        Arc::new(Slot::new(Block::new(root, true)?, irq(0))),
        Arc::new(Slot::new(Block::new(workspace, false)?, irq(1))),
        net.clone(),
        vsock.clone(),
        gpu.clone(),
        keyboard.clone(),
        tablet.clone(),
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
        gpu,
        keyboard,
        tablet,
        desktop,
        environment,
        cmdline: format!("{CMDLINE} {}{devices} -- guest", network.cmdline),
    })
}

/// Runs one vCPU until the guest shuts down, serving its port I/O and MMIO.
fn run_vcpu(
    vcpu: &OwnedFd,
    size: usize,
    vm: RawFd,
    bus: &[Arc<dyn Mmio>],
    serial: &Mutex<serial::Serial>,
    memory: &GuestMemory,
) -> io::Result<()> {
    let run = unsafe {
        mmap(
            None,
            NonZeroUsize::new(size).ok_or(Errno::EINVAL)?,
            ProtFlags::PROT_READ | ProtFlags::PROT_WRITE,
            MapFlags::MAP_SHARED,
            vcpu,
            0,
        )
    }
    .map_err(failed("mmap kvm_run"))?
    .cast::<sys::Run>();
    loop {
        // A waiting vCPU returns EAGAIN when its INIT arrives, and any vCPU EINTR on a signal.
        match unsafe { sys::run(vcpu.as_raw_fd(), 0) } {
            Err(Errno::EAGAIN | Errno::EINTR) => continue,
            result => result.map_err(failed("KVM_RUN"))?,
        };
        let run = unsafe { run.as_ptr().as_mut().ok_or(Errno::EFAULT)? };
        match run.exit_reason {
            sys::EXIT_IO => port_io(run, &mut serial.lock().expect("serial lock"), vm)?,
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

/// A size from the environment, or its default when unset.
fn sized(name: &str, (default, range): (u32, std::ops::RangeInclusive<u32>)) -> io::Result<u32> {
    let Some(value) = std::env::var_os(name) else {
        return Ok(default);
    };
    value
        .to_str()
        .and_then(|value| value.parse().ok())
        .filter(|value| range.contains(value))
        .ok_or_else(|| {
            let (low, high) = range.into_inner();
            io::Error::other(format!(
                "{name} must be a whole number from {low} to {high}"
            ))
        })
}

/// Maps the single guest memory region at guest physical address 0, with the in-kernel PIC, IOAPIC
/// and PIT the guest's timer and device interrupts need.
fn guest_memory(vm: &OwnedFd, size: usize) -> io::Result<GuestMemory> {
    let fd = vm.as_raw_fd();
    let memory = GuestMemory::map(size)?;
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

/// Creates the vCPUs with the host's supported CPUID and their own APIC IDs. The bootstrap
/// processor starts at the kernel entry; the rest wait in the in-kernel APIC for the guest's SIPI.
fn create_vcpus(kvm: &File, vm: RawFd, count: u8, entry: u64) -> io::Result<Vec<OwnedFd>> {
    let mut supported = sys::Cpuid {
        nent: 256,
        padding: 0,
        entries: [sys::CpuidEntry::default(); 256],
    };
    unsafe { sys::get_supported_cpuid(kvm.as_raw_fd(), &mut supported) }
        .map_err(failed("KVM_GET_SUPPORTED_CPUID"))?;
    let mut vcpus = Vec::new();
    for id in 0..count {
        let vcpu = unsafe {
            OwnedFd::from_raw_fd(
                sys::create_vcpu(vm, id.into()).map_err(failed("KVM_CREATE_VCPU"))?,
            )
        };
        let mut cpuid = supported.clone();
        as_vcpu(&mut cpuid, id);
        unsafe { sys::set_cpuid2(vcpu.as_raw_fd(), &cpuid) }.map_err(failed("KVM_SET_CPUID2"))?;
        vcpus.push(vcpu);
    }
    start_at(&vcpus[0], entry)?;
    Ok(vcpus)
}

/// Starts the vCPU in 64-bit long mode at the kernel entry.
fn start_at(vcpu: &OwnedFd, entry: u64) -> io::Result<()> {
    let fd = vcpu.as_raw_fd();
    let mut sregs = sys::Sregs::default();
    unsafe { sys::get_sregs(fd, &mut sregs) }.map_err(failed("KVM_GET_SREGS"))?;
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

/// The supported CPUID carries the host CPU's APIC ID; each vCPU reports its own, as the MP
/// table lists it.
fn as_vcpu(cpuid: &mut sys::Cpuid, id: u8) {
    for entry in cpuid.entries.iter_mut().take(cpuid.nent as usize) {
        match entry.function {
            0x1 => entry.ebx = entry.ebx & 0x00ff_ffff | u32::from(id) << 24,
            0xb | 0x1f => entry.edx = id.into(),
            // AMD's extended APIC ID and, one thread per core, the core ID.
            0x8000_001e => (entry.eax, entry.ebx) = (id.into(), id.into()),
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
    fn cpuid_reports_the_vcpus_apic_id_and_keeps_other_leaves() {
        let mut cpuid = sys::Cpuid {
            nent: 4,
            padding: 0,
            entries: [sys::CpuidEntry::default(); 256],
        };
        for (entry, function) in cpuid.entries.iter_mut().zip([0x1, 0xb, 0x7, 0x8000_001e]) {
            *entry = sys::CpuidEntry {
                function,
                ebx: 0x0201_0800,
                edx: 2,
                ..Default::default()
            };
        }
        as_vcpu(&mut cpuid, 3);
        assert_eq!(cpuid.entries[0].ebx, 0x0301_0800);
        assert_eq!(cpuid.entries[1].edx, 3);
        assert_eq!((cpuid.entries[3].eax, cpuid.entries[3].ebx), (3, 3));
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
