//! The guest's network (ADR 0084): a tap device in the sandbox's network namespace, joined to the
//! namespace's default-route interface by tc mirred redirects both ways, as Kata Containers'
//! tcfilter mode does. The guest takes that interface's MAC, IPv4 address and gateway, so pod
//! NetworkPolicy governs exactly the guest's traffic; the sandbox process keeps no network of its
//! own once the redirect is in place. It needs CAP_NET_ADMIN and `/dev/net/tun`.
use super::memory::invalid;
use nix::{ifaddrs::getifaddrs, ioctl_write_ptr_bad, request_code_write};
use std::{
    fs::{self, File},
    io,
    net::Ipv4Addr,
    os::fd::AsRawFd,
    path::Path,
    process::{Command, Stdio},
};

const TAP: &str = "tap0";
const IFF_TAP: i16 = 0x0002;
const IFF_NO_PI: i16 = 0x1000;

/// `struct ifreq` with the `ifr_flags` member of its union.
#[repr(C)]
struct InterfaceRequest {
    name: [u8; 16],
    flags: i16,
    padding: [u8; 22],
}

ioctl_write_ptr_bad!(
    set_tun_interface,
    request_code_write!(b'T', 202, size_of::<i32>()),
    InterfaceRequest
);

pub struct Network {
    pub tap: File,
    pub mac: [u8; 6],
    pub mtu: u16,
    /// The kernel `ip=` parameter that configures the guest's interface before init runs.
    pub cmdline: String,
}

pub fn attach() -> io::Result<Network> {
    let (link, gateway) = default_route(&fs::read_to_string("/proc/net/route")?)?;
    let (address, netmask) = ipv4(&link)?;
    let class = Path::new("/sys/class/net").join(&link);
    let mac = mac(&fs::read_to_string(class.join("address"))?)?;
    let mtu = fs::read_to_string(class.join("mtu"))?.trim().parse();
    let mtu = mtu.map_err(|_| invalid("unreadable interface MTU"))?;
    let resolver = fs::read_to_string("/etc/resolv.conf").unwrap_or_default();
    let tap = open_tap()?;
    redirect(&link, mtu)?;
    Ok(Network {
        tap,
        mac,
        mtu,
        cmdline: kernel_ip(address, gateway, netmask, &nameservers(&resolver)),
    })
}

/// The interface and gateway of the IPv4 default route in `/proc/net/route`.
fn default_route(table: &str) -> io::Result<(String, Option<Ipv4Addr>)> {
    for line in table.lines().skip(1) {
        let fields: Vec<_> = line.split_whitespace().collect();
        if let [link, "00000000", gateway, _, _, _, _, "00000000", ..] = fields[..] {
            // The kernel prints addresses as host-order hex words.
            let gateway = u32::from_str_radix(gateway, 16).map_err(|_| invalid("bad route"))?;
            let gateway =
                Some(Ipv4Addr::from(gateway.to_le_bytes())).filter(|g| !g.is_unspecified());
            return Ok((link.to_owned(), gateway));
        }
    }
    Err(io::Error::other(
        "the sandbox network namespace has no IPv4 default route",
    ))
}

fn ipv4(link: &str) -> io::Result<(Ipv4Addr, Ipv4Addr)> {
    getifaddrs()
        .map_err(io::Error::from)?
        .filter(|interface| interface.interface_name == link)
        .find_map(|interface| {
            let address = interface.address?.as_sockaddr_in()?.ip();
            Some((address, interface.netmask?.as_sockaddr_in()?.ip()))
        })
        .ok_or_else(|| io::Error::other(format!("{link} has no IPv4 address")))
}

fn mac(text: &str) -> io::Result<[u8; 6]> {
    let octets: Vec<_> = text
        .trim()
        .split(':')
        .map(|octet| u8::from_str_radix(octet, 16))
        .collect();
    octets
        .into_iter()
        .collect::<Result<Vec<_>, _>>()
        .ok()
        .and_then(|octets| octets.try_into().ok())
        .ok_or_else(|| invalid("unreadable interface MAC address"))
}

/// The first two IPv4 nameservers, which the guest kernel publishes in `/proc/net/pnp`.
fn nameservers(resolver: &str) -> Vec<Ipv4Addr> {
    resolver
        .lines()
        .filter_map(|line| line.strip_prefix("nameserver")?.trim().parse().ok())
        .take(2)
        .collect()
}

/// Kernel `ip=` adds the default route only through a gateway inside the subnet; for a /32 pod
/// address, as Calico gives, guest init adds it on-link from the same parameter.
// shortcut: IPv6 is not configured; add it once a dual-stack cluster is qualified.
fn kernel_ip(
    address: Ipv4Addr,
    gateway: Option<Ipv4Addr>,
    netmask: Ipv4Addr,
    dns: &[Ipv4Addr],
) -> String {
    let gateway = gateway
        .map(|gateway| gateway.to_string())
        .unwrap_or_default();
    let dns = |index: usize| dns.get(index).map(ToString::to_string).unwrap_or_default();
    format!(
        "ip={address}::{gateway}:{netmask}::eth0:off:{}:{}",
        dns(0),
        dns(1)
    )
}

fn open_tap() -> io::Result<File> {
    let tun = File::options()
        .read(true)
        .write(true)
        .open("/dev/net/tun")
        .map_err(|error| io::Error::other(format!("/dev/net/tun: {error}")))?;
    let mut request = InterfaceRequest {
        name: [0; 16],
        flags: IFF_TAP | IFF_NO_PI,
        padding: [0; 22],
    };
    request.name[..TAP.len()].copy_from_slice(TAP.as_bytes());
    unsafe { set_tun_interface(tun.as_raw_fd(), &request) }
        .map_err(|error| io::Error::other(format!("TUNSETIFF {TAP}: {error}")))?;
    Ok(tun)
}

/// Mirrors every frame arriving on either interface out of the other. The tap disappears with
/// the sandbox, so a restart first drops the stale ingress qdisc and its filter from `link`.
fn redirect(link: &str, mtu: u16) -> io::Result<()> {
    let mtu = mtu.to_string();
    let _ = Command::new("tc")
        .args(["qdisc", "del", "dev", link, "ingress"])
        .stderr(Stdio::null())
        .status();
    let commands: [&[&str]; 5] = [
        &["ip", "link", "set", TAP, "mtu", &mtu, "up"],
        &["tc", "qdisc", "add", "dev", TAP, "ingress"],
        &[
            "tc", "filter", "add", "dev", TAP, "parent", "ffff:", "matchall", "action", "mirred",
            "egress", "redirect", "dev", link,
        ],
        &["tc", "qdisc", "add", "dev", link, "ingress"],
        &[
            "tc", "filter", "add", "dev", link, "parent", "ffff:", "matchall", "action", "mirred",
            "egress", "redirect", "dev", TAP,
        ],
    ];
    for command in commands {
        let status = Command::new(command[0]).args(&command[1..]).status();
        let status =
            status.map_err(|error| io::Error::other(format!("{}: {error}", command[0])))?;
        if !status.success() {
            return Err(io::Error::other(format!("{}: {status}", command.join(" "))));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_default_route_and_resolvers() {
        let table = "Iface\tDestination\tGateway \tFlags\tRefCnt\tUse\tMetric\tMask\t\tMTU\tWindow\tIRTT\n\
            eth0\t0000C80A\t00000000\t0001\t0\t0\t0\t00FFFFFF\t0\t0\t0\n\
            eth0\t00000000\t0100C80A\t0003\t0\t0\t0\t00000000\t0\t0\t0\n";
        let (link, gateway) = default_route(table).unwrap();
        assert_eq!(
            (link.as_str(), gateway),
            ("eth0", Some(Ipv4Addr::new(10, 200, 0, 1)))
        );
        assert!(default_route(&table[..table.rfind("eth0").unwrap()]).is_err());
        let resolver = "search svc\nnameserver 10.96.0.10\nnameserver ::1\nnameserver 1.1.1.1\nnameserver 8.8.8.8\n";
        let dns = nameservers(resolver);
        assert_eq!(
            dns,
            [Ipv4Addr::new(10, 96, 0, 10), Ipv4Addr::new(1, 1, 1, 1)]
        );
        let address = Ipv4Addr::new(10, 200, 0, 2);
        let netmask = Ipv4Addr::new(255, 255, 255, 0);
        assert_eq!(
            kernel_ip(address, gateway, netmask, &dns),
            "ip=10.200.0.2::10.200.0.1:255.255.255.0::eth0:off:10.96.0.10:1.1.1.1"
        );
        assert_eq!(
            kernel_ip(address, None, netmask, &[]),
            "ip=10.200.0.2:::255.255.255.0::eth0:off::"
        );
        assert_eq!(
            mac("02:42:ac:11:00:02\n").unwrap(),
            [2, 0x42, 0xac, 0x11, 0, 2]
        );
        assert!(mac("02:42").is_err());
    }
}
