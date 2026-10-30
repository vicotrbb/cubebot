//! `cubebot-agent guest`: init of the `kvm` sandbox's guest (ADR 0084). It mounts the workspace
//! disk, receives the agent's environment over the control channel ([`cubebot_protocol::guest`]),
//! runs the agent as its child while reaping orphaned tool processes, rewrites received files the
//! host refreshes, and reports liveness and then the agent's exit status to the host. With
//! computer use on it starts the desktop (ADR 0085) as the tool user. When the
//! agent exits it syncs the disks and reboots, which ends the VM; the host ending its side of the
//! channel stops the agent first.
use cubebot_protocol::{
    Liveness, LivenessStatus,
    guest::{self, Report},
};
use nix::{
    errno::Errno,
    mount::{MsFlags, mount, umount},
    net::if_::if_nametoindex,
    sys::{
        reboot::{RebootMode, reboot},
        signal::{Signal, kill},
        socket::{
            AddressFamily, MsgFlags, NetlinkAddr, SockFlag, SockProtocol, SockType, VsockAddr,
            connect, recv, sendto, socket,
        },
        wait::{WaitStatus, waitpid},
    },
    unistd::{Pid, sync},
};
use std::{
    fs::{DirBuilder, File, OpenOptions},
    io::{self, BufRead, BufReader, Read, Write},
    net::Ipv4Addr,
    os::{
        fd::AsRawFd,
        unix::{
            fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
            process::CommandExt,
        },
    },
    path::Path,
    process::{Command, ExitCode, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

/// The workspace disk, laid out as the pod's workspace volume is.
const STATE: &str = "/var/lib/cubebot";
const JOURNAL: &str = "/var/lib/cubebot/runtime";
const WORKSPACE: &str = "/var/lib/cubebot/workspace";
/// Files received from the host, readable only by the agent.
const PRIVATE: &str = "/run/cubebot";
const PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
const REPORT_INTERVAL: Duration = Duration::from_secs(5);
const CHROMIUM: [&str; 9] = [
    "--no-sandbox",
    "--test-type",
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-gpu",
    "--start-maximized",
    "--password-store=basic",
    "--user-data-dir=/var/lib/cubebot/workspace/.config/chromium",
    "about:blank",
];

pub fn init() -> ExitCode {
    if let Err(error) = boot() {
        eprintln!("Guest init failed: {error}");
    }
    // Stop orphaned tools and unmount the workspace, so the disk is clean before the VM ends.
    let _ = kill(Pid::from_raw(-1), Signal::SIGKILL);
    while !matches!(waitpid(None, None), Err(Errno::ECHILD)) {}
    if let Err(error) = umount(STATE) {
        eprintln!("Guest workspace unmount failed: {error}");
    }
    sync();
    // Init must never exit; with reboot=t a reboot ends the VM.
    let Err(error) = reboot(RebootMode::RB_AUTOBOOT);
    eprintln!("Guest reboot failed: {error}");
    ExitCode::FAILURE
}

fn boot() -> io::Result<()> {
    mount_filesystems()?;
    default_route(&std::fs::read_to_string("/proc/cmdline")?)?;
    let channel = control_channel()?;
    let mut updates = BufReader::new(channel.try_clone()?);
    let environment = prepare(receive(&mut updates)?)?;
    desktop(&environment);
    let agent = Command::new(std::env::current_exe()?)
        .env_clear()
        .envs(environment)
        .spawn()?;
    let agent = Pid::from_raw(agent.id() as i32);
    let channel = Arc::new(Mutex::new(channel));
    report(channel.clone());
    follow(updates, agent);
    let exit = reap(agent)?;
    send(&channel, &Report::Exit { exit })
}

fn mount_filesystems() -> io::Result<()> {
    let private = MsFlags::MS_NOSUID | MsFlags::MS_NODEV;
    // The kernel's devtmpfs has neither; terminals and browsers need both.
    for directory in ["/dev/pts", "/dev/shm"] {
        DirBuilder::new().mode(0o755).create(directory)?;
    }
    let mounts = [
        ("proc", "/proc", "proc", private | MsFlags::MS_NOEXEC, None),
        ("sysfs", "/sys", "sysfs", private | MsFlags::MS_NOEXEC, None),
        (
            "tmpfs",
            "/run",
            "tmpfs",
            private | MsFlags::MS_NOEXEC,
            Some("mode=0755"),
        ),
        ("tmpfs", "/tmp", "tmpfs", private, Some("mode=1777")),
        ("tmpfs", "/dev/shm", "tmpfs", private, Some("mode=1777")),
        (
            "devpts",
            "/dev/pts",
            "devpts",
            MsFlags::MS_NOSUID | MsFlags::MS_NOEXEC,
            Some("mode=0620,ptmxmode=0666"),
        ),
        // The workspace disk is the second virtio disk, after the read-only root. Without delayed
        // allocation each journal commit writes file data first, so the commit that journals a
        // tool result also persists that tool's effects; the guest's page cache dies with the VM.
        ("/dev/vdb", STATE, "ext4", private, Some("nodelalloc")),
        // The kernel publishes the nameservers from its ip= parameter in resolv.conf syntax.
        (
            "/proc/net/pnp",
            "/etc/resolv.conf",
            "",
            MsFlags::MS_BIND,
            None,
        ),
    ];
    for (source, target, kind, flags, data) in mounts {
        mount(Some(source), target, Some(kind), flags, data)
            .map_err(|error| io::Error::other(format!("mount {target}: {error}")))?;
    }
    Ok(())
}

/// Sends every packet through the kernel `ip=` gateway, as pod CNIs route: kind's ptp routes even
/// its own subnet that way and answers no ARP for other pods, and a /32 address with Calico's
/// link-local gateway gets no default route from `ip=`, whose netmask the kernel then guesses.
/// So each on-link route the kernel added becomes the same prefix via the gateway, over netlink.
fn default_route(cmdline: &str) -> io::Result<()> {
    let Some((gateway, interface)) = gateway(cmdline) else {
        return Ok(());
    };
    let socket = socket(
        AddressFamily::Netlink,
        SockType::Raw,
        SockFlag::SOCK_CLOEXEC,
        SockProtocol::NetlinkRoute,
    )?;
    let index = if_nametoindex(interface)?;
    let table = std::fs::read_to_string("/proc/net/route")?;
    for destination in on_link(&table, interface)
        .into_iter()
        .chain([(Ipv4Addr::UNSPECIFIED, 0)])
    {
        let request = route_request(destination, gateway, index);
        sendto(
            socket.as_raw_fd(),
            &request,
            &NetlinkAddr::new(0, 0),
            MsgFlags::empty(),
        )?;
        let mut reply = [0; 64];
        let read = recv(socket.as_raw_fd(), &mut reply, MsgFlags::empty())?;
        // The acknowledgement is an NLMSG_ERROR message whose error is zero.
        let error = i32::from_ne_bytes(reply[16..20].try_into().expect("four bytes"));
        match (read >= 20, u16::from_ne_bytes([reply[4], reply[5]]), error) {
            (true, 2, 0) => (),
            (true, 2, error) => {
                return Err(io::Error::other(format!(
                    "route {}/{} via {gateway}: {}",
                    destination.0,
                    destination.1,
                    io::Error::from_raw_os_error(-error)
                )));
            }
            _ => return Err(io::Error::other("unexpected netlink reply")),
        }
    }
    Ok(())
}

/// The gateway and interface of the kernel `ip=` parameter.
fn gateway(cmdline: &str) -> Option<(Ipv4Addr, &str)> {
    let parameter = cmdline
        .split_whitespace()
        .find_map(|parameter| parameter.strip_prefix("ip="))?;
    let fields: Vec<_> = parameter.split(':').collect();
    let [_, _, gateway, _, _, interface, ..] = fields[..] else {
        return None;
    };
    Some((gateway.parse().ok()?, interface))
}

/// The prefixes `/proc/net/route` lists as on-link (no gateway) through `interface`.
fn on_link(table: &str, interface: &str) -> Vec<(Ipv4Addr, u8)> {
    table
        .lines()
        .skip(1)
        .filter_map(|line| {
            let fields: Vec<_> = line.split_whitespace().collect();
            let [link, destination, "00000000", _, _, _, _, mask, ..] = fields[..] else {
                return None;
            };
            // The kernel prints addresses as host-order hex words.
            let word = |hex| u32::from_str_radix(hex, 16).ok().map(u32::to_le_bytes);
            let mask = u32::from_be_bytes(word(mask)?);
            if link != interface || mask == 0 {
                return None;
            }
            Some((Ipv4Addr::from(word(destination)?), mask.count_ones() as u8))
        })
        .collect()
}

/// RTM_NEWROUTE replacing `destination via gateway dev interface onlink` in the main table.
fn route_request(destination: (Ipv4Addr, u8), gateway: Ipv4Addr, interface: u32) -> Vec<u8> {
    const NEW_ROUTE: u16 = 24;
    // NLM_F_REQUEST | NLM_F_ACK | NLM_F_REPLACE | NLM_F_CREATE
    const FLAGS: u16 = 0x505;
    const ON_LINK: u32 = 4;
    let length: u32 = if destination.1 == 0 { 44 } else { 52 };
    let mut message = Vec::with_capacity(length as usize);
    message.extend_from_slice(&length.to_ne_bytes());
    message.extend_from_slice(&NEW_ROUTE.to_ne_bytes());
    message.extend_from_slice(&FLAGS.to_ne_bytes());
    // Sequence and port id; the kernel fills in the port.
    message.extend_from_slice(&[0; 8]);
    // rtmsg: IPv4, the destination length, main table, boot protocol, universe scope, unicast.
    message.extend_from_slice(&[2, destination.1, 0, 0, 254, 3, 0, 1]);
    message.extend_from_slice(&ON_LINK.to_ne_bytes());
    // RTA_GATEWAY and RTA_OIF attributes, then RTA_DST unless the route is the default.
    message.extend_from_slice(&8u16.to_ne_bytes());
    message.extend_from_slice(&5u16.to_ne_bytes());
    message.extend_from_slice(&gateway.octets());
    message.extend_from_slice(&8u16.to_ne_bytes());
    message.extend_from_slice(&4u16.to_ne_bytes());
    message.extend_from_slice(&interface.to_ne_bytes());
    if destination.1 != 0 {
        message.extend_from_slice(&8u16.to_ne_bytes());
        message.extend_from_slice(&1u16.to_ne_bytes());
        message.extend_from_slice(&destination.0.octets());
    }
    message
}

fn control_channel() -> io::Result<File> {
    let socket = socket(
        AddressFamily::Vsock,
        SockType::Stream,
        SockFlag::SOCK_CLOEXEC,
        None,
    )?;
    connect(
        socket.as_raw_fd(),
        &VsockAddr::new(guest::HOST_CID, guest::PORT),
    )
    .map_err(|error| io::Error::other(format!("control channel: {error}")))?;
    Ok(File::from(socket))
}

/// Reads one message, leaving any that follow it in `channel`.
fn receive(channel: &mut BufReader<File>) -> io::Result<Vec<(String, String)>> {
    let mut message = Vec::new();
    while !guest::complete(&message) {
        let limit = (guest::ENVIRONMENT_LIMIT + 1 - message.len()) as u64;
        if channel.by_ref().take(limit).read_until(0, &mut message)? == 0
            || message.len() > guest::ENVIRONMENT_LIMIT
        {
            return Err(io::Error::other("incomplete guest environment"));
        }
    }
    guest::decode(&message).ok_or_else(|| io::Error::other("invalid guest environment"))
}

/// The agent's environment: the host's settings with each received file written privately, then
/// the guest's own paths, which win over any duplicate.
fn prepare(received: Vec<(String, String)>) -> io::Result<Vec<(String, String)>> {
    DirBuilder::new().mode(0o700).create(PRIVATE)?;
    let mut environment = Vec::new();
    let mut tool_user = None;
    for (name, value) in received {
        validate(&name)?;
        if name == "CUBEBOT_TOOL_USER" {
            tool_user = Some(value.clone());
        }
        let value = match name.ends_with("_FILE") {
            true => private_file(&name, &value)?,
            false => value,
        };
        environment.push((name, value));
    }
    storage(tool_user.as_deref())?;
    environment.extend([
        ("PATH".to_owned(), PATH.to_owned()),
        ("CUBEBOT_JOURNAL_DIRECTORY".to_owned(), JOURNAL.to_owned()),
        (
            "CUBEBOT_WORKSPACE_DIRECTORY".to_owned(),
            WORKSPACE.to_owned(),
        ),
    ]);
    Ok(environment)
}

fn validate(name: &str) -> io::Result<()> {
    if name.is_empty()
        || !name
            .bytes()
            .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(io::Error::other("invalid guest environment name"));
    }
    Ok(())
}

/// Writes the file for a `*_FILE` entry whole, so the agent never reads half of a refresh.
fn private_file(name: &str, contents: &str) -> io::Result<String> {
    let path = Path::new(PRIVATE).join(name.to_lowercase());
    let partial = path.with_extension("partial");
    OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&partial)?
        .write_all(contents.as_bytes())?;
    std::fs::rename(&partial, &path)?;
    Ok(path.to_string_lossy().into_owned())
}

/// The agent's private journal, and a workspace the tool user can write.
fn storage(tool_user: Option<&str>) -> io::Result<()> {
    DirBuilder::new().recursive(true).create(JOURNAL)?;
    std::fs::set_permissions(JOURNAL, std::fs::Permissions::from_mode(0o700))?;
    DirBuilder::new()
        .recursive(true)
        .mode(0o755)
        .create(WORKSPACE)?;
    let owner = tool_user
        .and_then(|user| user.split_once(':'))
        .and_then(|(uid, gid)| Some((uid.parse().ok()?, gid.parse().ok()?)));
    // The agent refuses a malformed or root tool user itself, and then runs no tools.
    if let Some((uid, gid)) = owner.filter(|&(uid, gid)| uid != 0 && gid != 0) {
        std::os::unix::fs::chown(WORKSPACE, Some(uid), Some(gid))?;
    }
    Ok(())
}

/// Starts the desktop when computer use is on and a tool user is configured. A desktop that
/// fails to start leaves the agent running without a screen.
fn desktop(environment: &[(String, String)]) {
    let setting = |wanted: &str| {
        environment
            .iter()
            .find(|(name, _)| name == wanted)
            .map(|(_, value)| value.as_str())
    };
    let user = setting("CUBEBOT_TOOL_USER")
        .and_then(|user| user.split_once(':'))
        .and_then(|(uid, gid)| Some((uid.parse().ok()?, gid.parse().ok()?)))
        .filter(|&(uid, gid)| uid != 0 && gid != 0);
    let (Some("1"), Some((uid, gid))) = (setting("CUBEBOT_COMPUTER_ENABLED"), user) else {
        return;
    };
    std::thread::spawn(move || {
        if let Err(error) = start_desktop(uid, gid) {
            eprintln!("Guest desktop failed: {error}");
        }
    });
}

/// Xorg on the virtio-gpu display with the virtio keyboard (event0) and tablet (event1), which
/// the kernel probes in the sandbox's device order, then openbox, xterm and Chromium, all as the
/// tool user. With no udev or logind in the guest, Xorg opens the devices and its VT itself.
fn start_desktop(uid: u32, gid: u32) -> io::Result<()> {
    let devices = [
        "/dev/dri/card0",
        "/dev/input/event0",
        "/dev/input/event1",
        "/dev/tty0",
        "/dev/tty1",
    ];
    for device in devices {
        std::os::unix::fs::chown(device, Some(uid), Some(gid))
            .map_err(|error| io::Error::other(format!("{device}: {error}")))?;
    }
    let client = |program: &str| {
        let mut command = Command::new(program);
        command
            .env_clear()
            .envs([
                ("HOME", WORKSPACE),
                ("PATH", PATH),
                ("DISPLAY", ":0"),
                ("LANG", "C.UTF-8"),
            ])
            .current_dir(WORKSPACE)
            .uid(uid)
            .gid(gid)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        command
    };
    // VT ioctls need tty1 as the caller's controlling terminal, which `setsid -c` makes it and
    // `-keeptty` keeps. Xorg's errors reach the console; its log is in ~/.local/share/xorg.
    let terminal = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty1")?;
    client("/usr/bin/setsid")
        .args([
            "-c",
            "/usr/lib/xorg/Xorg",
            ":0",
            "vt1",
            "-keeptty",
            "-nolisten",
            "tcp",
            "-noreset",
        ])
        .stdin(terminal)
        .stderr(Stdio::inherit())
        .spawn()?;
    let display = Path::new("/tmp/.X11-unix/X0");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !display.exists() {
        if std::time::Instant::now() > deadline {
            return Err(io::Error::other("Xorg did not open display :0"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    client("/usr/bin/openbox").spawn()?;
    client("/usr/bin/xterm").spawn()?;
    // The VM bounds Chromium, and the guest kernel has no user namespaces for its own sandbox.
    client("/usr/bin/chromium").args(CHROMIUM).spawn()?;
    eprintln!("Guest desktop started");
    Ok(())
}

/// One whole line per report, so liveness and the exit status never interleave.
fn send(channel: &Mutex<File>, report: &Report) -> io::Result<()> {
    let mut line = serde_json::to_vec(report).expect("report serializes");
    line.push(b'\n');
    channel.lock().expect("channel lock").write_all(&line)
}

fn report(channel: Arc<Mutex<File>>) {
    let alive = Report::Liveness(Liveness {
        status: LivenessStatus::Alive,
    });
    std::thread::spawn(move || {
        while send(&channel, &alive).is_ok() {
            std::thread::sleep(REPORT_INTERVAL);
        }
    });
}

/// Rewrites the files the host refreshes; the end of the host's side means stop.
fn follow(mut channel: BufReader<File>, agent: Pid) {
    std::thread::spawn(move || {
        if let Err(error) = refresh(&mut channel) {
            eprintln!("Guest control channel ended: {error}");
        }
        let _ = kill(agent, Signal::SIGTERM);
    });
}

fn refresh(channel: &mut BufReader<File>) -> io::Result<()> {
    while !channel.fill_buf()?.is_empty() {
        for (name, value) in receive(channel)? {
            validate(&name)?;
            if name.ends_with("_FILE") {
                private_file(&name, &value)?;
            }
        }
    }
    Ok(())
}

/// As PID 1, waits for every child, orphaned tool processes included, until the agent exits, and
/// returns its exit status as a shell reports it.
fn reap(agent: Pid) -> io::Result<u8> {
    loop {
        match waitpid(None, None) {
            Ok(WaitStatus::Exited(pid, code)) if pid == agent => {
                eprintln!("Agent exited with status {code}");
                return Ok(code as u8);
            }
            Ok(WaitStatus::Signaled(pid, signal, _)) if pid == agent => {
                eprintln!("Agent ended by {signal}");
                return Ok(128 + signal as u8);
            }
            Ok(_) | Err(Errno::EINTR) => (),
            Err(error) => return Err(error.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn routes_every_prefix_through_the_gateway() {
        let calico = "console=ttyS0 ip=10.0.5.7::169.254.1.1:255.255.255.255::eth0:off:: ro";
        let gateway_address = Ipv4Addr::new(169, 254, 1, 1);
        assert_eq!(gateway(calico), Some((gateway_address, "eth0")));
        let ptp = "ip=10.244.0.7::10.244.0.1:255.255.255.0::eth0:off::";
        assert_eq!(gateway(ptp), Some((Ipv4Addr::new(10, 244, 0, 1), "eth0")));
        assert_eq!(gateway("ip=10.0.5.7:::255.255.255.255::eth0:off::"), None);
        assert_eq!(gateway("console=ttyS0"), None);
        let table = "Iface\tDestination\tGateway\tFlags\tRefCnt\tUse\tMetric\tMask\n\
            eth0\t00000000\t0100F40A\t0003\t0\t0\t0\t00000000\t0\t0\t0\n\
            eth0\t0000F40A\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n\
            lo\t0000007F\t00000000\t0001\t0\t0\t0\t000000FF\t0\t0\t0\n";
        assert_eq!(on_link(table, "eth0"), [(Ipv4Addr::new(10, 244, 0, 0), 24)]);
        let request = route_request((Ipv4Addr::UNSPECIFIED, 0), gateway_address, 2);
        assert_eq!(request.len(), 44);
        assert_eq!(request[..4], 44u32.to_ne_bytes());
        assert_eq!(request[32..36], [169, 254, 1, 1]);
        assert_eq!(request[40..], 2u32.to_ne_bytes());
        let request = route_request((Ipv4Addr::new(10, 244, 0, 0), 24), gateway_address, 2);
        assert_eq!((request.len(), request[17]), (52, 24));
        assert_eq!(request[44..48], [8, 0, 1, 0]);
        assert_eq!(request[48..], [10, 244, 0, 0]);
    }
}
