//! `cubebot-agent guest`: init of the `kvm` sandbox's guest (ADR 0084). It mounts the workspace
//! disk, receives the agent's environment over the control channel ([`cubebot_protocol::guest`]),
//! runs the agent as its child while reaping orphaned tool processes, rewrites received files the
//! host refreshes, and reports liveness and then the agent's exit status to the host. When the
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
        unix::fs::{DirBuilderExt, OpenOptionsExt, PermissionsExt},
    },
    path::Path,
    process::{Command, ExitCode},
    sync::{Arc, Mutex},
    time::Duration,
};

/// The workspace disk, laid out as the `process` backend's volume is.
const STATE: &str = "/var/lib/cubebot";
const JOURNAL: &str = "/var/lib/cubebot/runtime";
const WORKSPACE: &str = "/var/lib/cubebot/workspace";
/// Files received from the host, readable only by the agent.
const PRIVATE: &str = "/run/cubebot";
const PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
const REPORT_INTERVAL: Duration = Duration::from_secs(5);

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
        // The workspace disk is the second virtio disk, after the read-only root.
        ("/dev/vdb", STATE, "ext4", private, None),
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

/// Kernel `ip=` adds no default route through a gateway outside the address's subnet, as a /32 pod
/// address with Calico's link-local gateway has; this adds that route on-link over netlink.
fn default_route(cmdline: &str) -> io::Result<()> {
    let Some((gateway, interface)) = off_link_gateway(cmdline) else {
        return Ok(());
    };
    let socket = socket(
        AddressFamily::Netlink,
        SockType::Raw,
        SockFlag::SOCK_CLOEXEC,
        SockProtocol::NetlinkRoute,
    )?;
    let request = route_request(gateway, if_nametoindex(interface)?);
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
        (true, 2, 0) => Ok(()),
        (true, 2, error) => Err(io::Error::other(format!(
            "default route via {gateway}: {}",
            io::Error::from_raw_os_error(-error)
        ))),
        _ => Err(io::Error::other("unexpected netlink reply")),
    }
}

/// The gateway and interface of the kernel `ip=` parameter, when the gateway is off the subnet.
fn off_link_gateway(cmdline: &str) -> Option<(Ipv4Addr, &str)> {
    let parameter = cmdline
        .split_whitespace()
        .find_map(|parameter| parameter.strip_prefix("ip="))?;
    let fields: Vec<_> = parameter.split(':').collect();
    let [address, _, gateway, netmask, _, interface, ..] = fields[..] else {
        return None;
    };
    let address = u32::from(address.parse::<Ipv4Addr>().ok()?);
    let gateway = gateway.parse::<Ipv4Addr>().ok()?;
    let netmask = u32::from(netmask.parse::<Ipv4Addr>().ok()?);
    ((address ^ u32::from(gateway)) & netmask != 0).then_some((gateway, interface))
}

/// RTM_NEWROUTE for `default via gateway dev interface onlink` in the main table.
fn route_request(gateway: Ipv4Addr, interface: u32) -> Vec<u8> {
    const LENGTH: u32 = 44;
    const NEW_ROUTE: u16 = 24;
    // NLM_F_REQUEST | NLM_F_ACK | NLM_F_EXCL | NLM_F_CREATE
    const FLAGS: u16 = 0x605;
    const ON_LINK: u32 = 4;
    let mut message = Vec::with_capacity(LENGTH as usize);
    message.extend_from_slice(&LENGTH.to_ne_bytes());
    message.extend_from_slice(&NEW_ROUTE.to_ne_bytes());
    message.extend_from_slice(&FLAGS.to_ne_bytes());
    // Sequence and port id; the kernel fills in the port.
    message.extend_from_slice(&[0; 8]);
    // rtmsg: IPv4, a /0 destination, main table, boot protocol, universe scope, unicast.
    message.extend_from_slice(&[2, 0, 0, 0, 254, 3, 0, 1]);
    message.extend_from_slice(&ON_LINK.to_ne_bytes());
    // RTA_GATEWAY and RTA_OIF attributes.
    message.extend_from_slice(&8u16.to_ne_bytes());
    message.extend_from_slice(&5u16.to_ne_bytes());
    message.extend_from_slice(&gateway.octets());
    message.extend_from_slice(&8u16.to_ne_bytes());
    message.extend_from_slice(&4u16.to_ne_bytes());
    message.extend_from_slice(&interface.to_ne_bytes());
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
    fn routes_only_an_off_link_gateway() {
        let calico = "console=ttyS0 ip=10.0.5.7::169.254.1.1:255.255.255.255::eth0:off:: ro";
        let gateway = Ipv4Addr::new(169, 254, 1, 1);
        assert_eq!(off_link_gateway(calico), Some((gateway, "eth0")));
        let bridge = "ip=10.0.5.7::10.0.5.1:255.255.255.0::eth0:off::";
        assert_eq!(off_link_gateway(bridge), None);
        assert_eq!(
            off_link_gateway("ip=10.0.5.7:::255.255.255.255::eth0:off::"),
            None
        );
        assert_eq!(off_link_gateway("console=ttyS0"), None);
        let request = route_request(gateway, 2);
        assert_eq!(request.len(), 44);
        assert_eq!(request[..4], 44u32.to_ne_bytes());
        assert_eq!(request[32..36], [169, 254, 1, 1]);
        assert_eq!(request[40..], 2u32.to_ne_bytes());
    }
}
