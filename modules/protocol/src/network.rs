//! Non-secret installation network configuration shared with the trusted Pod initializer.
use ipnet::IpNet;
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "RawNetworkPolicy")]
pub struct NetworkPolicy {
    infrastructure: Vec<IpNet>,
    dns: Vec<IpAddr>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNetworkPolicy {
    infrastructure: Vec<IpNet>,
    dns: Vec<IpAddr>,
}
impl NetworkPolicy {
    pub fn new(infrastructure: Vec<IpNet>, dns: Vec<IpAddr>) -> Result<Self, &'static str> {
        if infrastructure.len() > 128 || dns.len() > 3 {
            return Err("network configuration exceeds limits");
        }
        for prefix in &infrastructure {
            // Mapped socket destinations are IPv4 packets, so require an IPv4 CIDR.
            if prefix.addr() != prefix.network()
                || matches!(prefix.addr(), IpAddr::V6(address) if address.to_ipv4_mapped().is_some())
            {
                return Err("infrastructure prefix must be canonical and unmapped");
            }
        }
        Ok(Self {
            infrastructure,
            dns: dns
                .into_iter()
                .map(|address| address.to_canonical())
                .collect(),
        })
    }
    pub fn infrastructure(&self) -> &[IpNet] {
        &self.infrastructure
    }
    pub fn dns(&self) -> &[IpAddr] {
        &self.dns
    }
}
impl TryFrom<RawNetworkPolicy> for NetworkPolicy {
    type Error = &'static str;
    fn try_from(value: RawNetworkPolicy) -> Result<Self, Self::Error> {
        Self::new(value.infrastructure, value.dns)
    }
}
