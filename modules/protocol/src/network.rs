//! Non-secret installation network configuration for agent sandboxes.
use serde::{Deserialize, Serialize};
use std::net::IpAddr;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(try_from = "RawNetworkPolicy")]
pub struct NetworkPolicy {
    dns: Vec<IpAddr>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNetworkPolicy {
    dns: Vec<IpAddr>,
}
impl NetworkPolicy {
    pub fn new(dns: Vec<IpAddr>) -> Result<Self, &'static str> {
        if dns.len() > 3 {
            return Err("network configuration exceeds limits");
        }
        Ok(Self {
            dns: dns
                .into_iter()
                .map(|address| address.to_canonical())
                .collect(),
        })
    }
    pub fn dns(&self) -> &[IpAddr] {
        &self.dns
    }
}
impl TryFrom<RawNetworkPolicy> for NetworkPolicy {
    type Error = &'static str;
    fn try_from(value: RawNetworkPolicy) -> Result<Self, Self::Error> {
        Self::new(value.dns)
    }
}
