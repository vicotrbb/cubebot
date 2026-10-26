//! Trusted network setup. Call only before tool processes start, with namespace NET_ADMIN.
use ipnet::IpNet;
use serde::Deserialize;
use std::{net::IpAddr, process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Command};

// IANA special-purpose registries, checked 2026-10-26. Include globally reachable
// special services as restricted too. IPv6 outside 2000::/3 is not public here.
const SPECIAL: &[&str] = &[
    "0.0.0.0/8",
    "10.0.0.0/8",
    "100.64.0.0/10",
    "127.0.0.0/8",
    "169.254.0.0/16",
    "172.16.0.0/12",
    "192.0.0.0/24",
    "192.0.2.0/24",
    "192.31.196.0/24",
    "192.52.193.0/24",
    "192.88.99.0/24",
    "192.168.0.0/16",
    "192.175.48.0/24",
    "198.18.0.0/15",
    "198.51.100.0/24",
    "203.0.113.0/24",
    "224.0.0.0/3",
    "2001::/23",
    "2001:db8::/32",
    "2002::/16",
    "2620:4f:8000::/48",
    "3fff::/20",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Configuration {
    infrastructure: Vec<String>,
    dns: Vec<IpAddr>,
}
pub struct Policy {
    blocked: Vec<IpNet>,
    dns: Vec<IpAddr>,
}
impl Policy {
    /// Topology includes public-address infrastructure, cluster/service/node ranges,
    /// metadata, translation and tunnel destinations. Never infer it from a hostname.
    pub fn parse(input: &str) -> Result<Self, &'static str> {
        if input.len() > 16384 {
            return Err("network configuration too large");
        }
        let config: Configuration =
            serde_json::from_str(input).map_err(|_| "invalid network configuration")?;
        if config.infrastructure.len() > 128 || config.dns.len() > 4 {
            return Err("network configuration exceeds limits");
        }
        let mut blocked = SPECIAL
            .iter()
            .map(|value| value.parse().expect("fixed CIDR"))
            .collect::<Vec<IpNet>>();
        for value in config.infrastructure {
            let prefix: IpNet = value.parse().map_err(|_| "invalid infrastructure prefix")?;
            if prefix.addr() != prefix.network() {
                return Err("infrastructure prefix must be canonical");
            }
            blocked.push(prefix);
        }
        Ok(Self {
            blocked,
            dns: config.dns,
        })
    }
    pub fn rules(&self) -> String {
        // add is idempotent; delete/recreate affects only this table, in one nft batch.
        let mut rules = String::from(
            "add table inet cubebot_network\ndelete table inet cubebot_network\ntable inet cubebot_network {\n",
        );
        for (family, kind) in [(4, "ipv4_addr"), (6, "ipv6_addr")] {
            let blocked = self
                .blocked
                .iter()
                .filter(|prefix| prefix.addr().is_ipv4() == (family == 4))
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            let dns = self
                .dns
                .iter()
                .filter(|address| address.is_ipv4() == (family == 4))
                .map(ToString::to_string)
                .collect::<Vec<_>>();
            rules += &set(
                &format!("blocked{family}"),
                kind,
                "flags interval; auto-merge;",
                blocked,
            );
            rules += &set(&format!("dns{family}"), kind, "", dns);
        }
        rules += &flow("tools_out", "daddr", "dport");
        rules += &flow("tools_in", "saddr", "sport");
        rules += CHAINS;
        rules += "}\n";
        rules
    }
}
fn set(name: &str, kind: &str, flags: &str, elements: Vec<String>) -> String {
    let entries = if elements.is_empty() {
        String::new()
    } else {
        format!("elements = {{ {} }};", elements.join(", "))
    };
    format!("set {name} {{ type {kind}; {flags} {entries} }}\n")
}
fn flow(name: &str, address: &str, port: &str) -> String {
    format!(
        "chain {name} {{\n\
        ip {address} @dns4 meta l4proto {{ tcp, udp }} th {port} 53 accept\n\
        ip6 {address} @dns6 meta l4proto {{ tcp, udp }} th {port} 53 accept\n\
        ip {address} @blocked4 drop\n\
        ip6 {address} @blocked6 drop\n\
        ip6 {address} != 2000::/3 drop\n\
        meta nfproto {{ ipv4, ipv6 }} meta l4proto {{ tcp, udp }} accept\n\
        }}\n"
    )
}
const CHAINS: &str = "chain output { type filter hook output priority 0; policy drop;
 ct direction original meta skuid 10001 ct mark set 0xcb01
 ct direction original meta skuid 10002 ct mark set 0xcb02
 meta skuid 10001 accept
 meta skuid 10002 ct direction original jump tools_out
 ip6 hoplimit 255 icmpv6 type { nd-neighbor-solicit, nd-neighbor-advert } accept
}
chain input { type filter hook input priority 0; policy drop;
 ct mark 0xcb01 accept
 ct mark 0xcb02 ct direction original jump tools_out
 ct mark 0xcb02 ct direction reply jump tools_in
 meta l4proto { 1, 58 } ct state related accept
 ip6 hoplimit 255 icmpv6 type { nd-neighbor-solicit, nd-neighbor-advert } accept
}
chain forward { type filter hook forward priority 0; policy drop; }
";

pub async fn initialize(input: &str) -> Result<(), &'static str> {
    let rules = Policy::parse(input)?.rules();
    tokio::time::timeout(Duration::from_secs(5), apply(rules))
        .await
        .map_err(|_| "network setup timed out")?
}
async fn apply(rules: String) -> Result<(), &'static str> {
    let mut child = Command::new("/usr/sbin/nft")
        .args(["-f", "-"])
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|_| "network setup unavailable")?;
    let mut input = child
        .stdin
        .take()
        .ok_or("network setup input unavailable")?;
    input
        .write_all(rules.as_bytes())
        .await
        .map_err(|_| "network setup write failed")?;
    drop(input);
    if !child
        .wait()
        .await
        .map_err(|_| "network setup failed")?
        .success()
    {
        return Err("network setup rejected");
    }
    Ok(())
}
