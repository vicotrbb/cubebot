//! What the watchdog (docs/adr/agents.md#a-watchdog-that-notifies) is told from inside and around
//! a sandbox: the agent's sensor reports sightings over its runtime channel, and the VMM keeps
//! a summary of the guest's network traffic that the orchestrator reads through the platform's
//! exec authorization.
use serde::{Deserialize, Serialize};
use std::net::Ipv4Addr;

/// An address on the internet: not private, shared (CGNAT), loopback, link-local, multicast,
/// broadcast or unspecified. Traffic to the others is the network policy's to judge.
pub fn public(address: Ipv4Addr) -> bool {
    let [first, second, ..] = address.octets();
    !(address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_multicast()
        || address.is_broadcast()
        || address.is_unspecified()
        || (first == 100 && (64..128).contains(&second)))
}

/// A process, as the sensor read it from `/proc`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Process {
    pub pid: u32,
    /// The executable's path, or the command's first word when the path cannot be read.
    pub executable: String,
    /// The command line, at most 300 characters.
    pub command: String,
}

/// Something the sensor saw since its previous report; each is reported once.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Sighting {
    /// Running as root with a real user that is not root, or under a process that is not root.
    Escalated(Process),
    /// A miner's name in a process that is not root.
    Miner(Process),
    /// A process that is not root, run from a temporary directory or from a deleted file.
    Hidden(Process),
    /// Processes that are not root, past a count ordinary work reaches.
    Flood { processes: u32 },
    /// A connection to a private address left unanswered, as the network policy leaves it.
    Blocked { address: String, port: u16 },
    /// A connection to a port that mail spam, miners, remote shells or Tor use.
    OddPort { address: String, port: u16 },
    /// A file only root can change, changed or created since the sensor started.
    Tampered { path: String },
    /// A root-owned setuid or setgid file where tools write.
    Setuid { path: String },
}

/// The guest's traffic since the previous read, kept by the VMM, which the guest cannot reach.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Traffic {
    /// How long this summary covers.
    pub seconds: u64,
    /// Per remote address, port and protocol.
    pub flows: Vec<Flow>,
    /// Names the guest asked DNS for, with how often.
    pub names: Vec<Name>,
    /// Public addresses the guest opened TCP connections to without DNS having returned them.
    pub unresolved: Vec<Endpoint>,
    /// Flows, names or endpoints left out because the summary was full.
    pub overflow: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Flow {
    pub address: String,
    pub port: u16,
    /// `tcp` or `udp`.
    pub protocol: String,
    /// Bytes of frames the guest sent to it, and received from it.
    pub sent: u64,
    pub received: u64,
    /// TCP connections the guest opened to it.
    pub opened: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Name {
    pub name: String,
    pub count: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub address: String,
    pub port: u16,
}
