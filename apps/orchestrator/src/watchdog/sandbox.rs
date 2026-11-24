//! What the watchdog learns from inside and around each agent's sandbox: the sensor's sightings
//! over the runtime channel and the VMM's traffic summary, read through the platform's exec
//! authorization (docs/adr/agents.md#a-watchdog-that-notifies).
use super::rules::{Hit, excerpt};
use crate::agents::{
    AgentId,
    sandbox::{ExecChannel, Sandbox},
};
use cubebot_protocol::watch::{Flow, Name, Sighting, Traffic, public};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    future::Future,
    net::Ipv4Addr,
    pin::Pin,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::io::AsyncReadExt;

const WINDOW: Duration = Duration::from_secs(5 * 60);
/// The sensor reports every few seconds; this much quiet on a live connection is silence.
const SILENCE: Duration = Duration::from_secs(60);
/// Distinct private addresses refused within the window that make a scan.
const SCAN: usize = 3;
const MIB: u64 = 1 << 20;
const LARGE: u64 = 100 * MIB;
const IDLE: u64 = 50 * MIB;
const LABEL: usize = 50;
const NAMES_PER_DOMAIN: usize = 150;
const READ_LIMIT: u64 = 4 << 20;
const READ_TIMEOUT: Duration = Duration::from_secs(10);

/// What one runtime channel remembers of its agent's sensor.
#[derive(Default)]
pub struct Sensed {
    heard: Option<Instant>,
    silent: bool,
    blocked: VecDeque<(Instant, (String, u16))>,
    scanned: bool,
}

impl Sensed {
    pub fn report(&mut self, sightings: &[Sighting]) -> Vec<Hit> {
        self.heard = Some(Instant::now());
        self.silent = false;
        sightings
            .iter()
            .filter_map(|sighting| self.sighting(sighting))
            .collect()
    }

    /// Once per silence, and only after the sensor has spoken on this connection, so an agent
    /// from before the sensor never counts as silent.
    pub fn silence(&mut self) -> Option<Hit> {
        let quiet = self.heard?.elapsed() > SILENCE && !self.silent;
        self.silent |= quiet;
        quiet.then(|| Hit {
            rule: "sensor_silent",
            severity: "medium",
            summary: "Its sandbox sensor stopped reporting while the agent stayed connected".into(),
            evidence: None,
        })
    }

    fn sighting(&mut self, sighting: &Sighting) -> Option<Hit> {
        let hit = |rule, severity, summary: String, evidence: Option<String>| Hit {
            rule,
            severity,
            summary,
            evidence,
        };
        Some(match sighting {
            Sighting::Escalated(process) => hit(
                "privilege_escalation",
                "high",
                format!(
                    "Ran `{}` as root from an unprivileged process",
                    short(&process.executable)
                ),
                Some(excerpt(&process.command)),
            ),
            Sighting::Miner(process) => hit(
                "miner_process",
                "high",
                format!(
                    "Runs a cryptocurrency miner as `{}`",
                    short(&process.executable)
                ),
                Some(excerpt(&process.command)),
            ),
            Sighting::Hidden(process) => hit(
                "hidden_executable",
                "low",
                format!(
                    "Ran `{}` from a temporary directory or a deleted file",
                    short(&process.executable)
                ),
                Some(excerpt(&process.command)),
            ),
            Sighting::Flood { processes } => hit(
                "process_flood",
                "medium",
                format!("Ran {processes} processes at once"),
                None,
            ),
            Sighting::Blocked { address, port } => return Some(self.blocked(address, *port)),
            Sighting::OddPort { address, port } => hit(
                "odd_port",
                "medium",
                format!(
                    "Connected to {address}:{port}, a port mail spam, miners, remote shells or Tor use"
                ),
                None,
            ),
            Sighting::Tampered { path } => hit(
                "tampered",
                "high",
                format!("A file only root can change was changed: {}", short(path)),
                None,
            ),
            Sighting::Setuid { path } => hit(
                "setuid_file",
                "high",
                format!(
                    "A root-owned setuid file appeared where tools write: {}",
                    short(path)
                ),
                None,
            ),
        })
    }

    fn blocked(&mut self, address: &str, port: u16) -> Hit {
        self.blocked
            .push_back((Instant::now(), (address.to_owned(), port)));
        while self
            .blocked
            .front()
            .is_some_and(|(at, _)| at.elapsed() > WINDOW)
        {
            self.blocked.pop_front();
        }
        let distinct: HashSet<&(String, u16)> = self.blocked.iter().map(|(_, to)| to).collect();
        self.scanned &= distinct.len() >= SCAN;
        if distinct.len() >= SCAN && !self.scanned {
            self.scanned = true;
            return Hit {
                rule: "lan_scan",
                severity: "high",
                summary: format!(
                    "Tried {} private addresses the network policy blocks within five minutes, the shape of a network scan",
                    distinct.len()
                ),
                evidence: Some(excerpt(
                    &distinct
                        .iter()
                        .map(|(address, port)| format!("{address}:{port}"))
                        .collect::<Vec<_>>()
                        .join(" "),
                )),
            };
        }
        Hit {
            rule: "blocked_private",
            severity: "low",
            summary: format!("Tried {address}:{port}, a private address the network policy blocks"),
            evidence: None,
        }
    }
}

fn short(text: &str) -> String {
    text.chars().take(120).collect()
}

/// The rules over one read of an agent's traffic; `idle` when it had no work in the window.
pub fn traffic(traffic: &Traffic, idle: bool) -> Vec<Hit> {
    let minutes = (traffic.seconds / 60).max(1);
    let outward: Vec<&Flow> = traffic
        .flows
        .iter()
        .filter(|flow| flow.address.parse::<Ipv4Addr>().is_ok_and(public))
        .collect();
    let mut hits = Vec::new();
    if let Some(flow) = outward
        .iter()
        .filter(|flow| flow.sent >= LARGE)
        .max_by_key(|flow| flow.sent)
    {
        hits.push(Hit {
            rule: "large_upload",
            severity: "medium",
            summary: format!(
                "Sent {} MB to {}:{} in {minutes} minutes",
                flow.sent / MIB,
                flow.address,
                flow.port
            ),
            evidence: None,
        });
    }
    let sent: u64 = outward.iter().map(|flow| flow.sent).sum();
    if idle && sent >= IDLE {
        hits.push(Hit {
            rule: "idle_traffic",
            severity: "medium",
            summary: format!(
                "Sent {} MB to the internet in {minutes} minutes with no work",
                sent / MIB
            ),
            evidence: None,
        });
    }
    hits.extend(tunnel(&traffic.names, minutes));
    if let Some(first) = traffic.unresolved.first() {
        hits.push(Hit {
            rule: "direct_ip",
            severity: "low",
            summary: format!(
                "Connected to {} public {} no DNS answer named, such as {}:{}",
                traffic.unresolved.len(),
                if traffic.unresolved.len() == 1 {
                    "address"
                } else {
                    "addresses"
                },
                first.address,
                first.port
            ),
            evidence: None,
        });
    }
    hits
}

/// A name with a label long enough to carry data, or very many names under one domain.
fn tunnel(names: &[Name], minutes: u64) -> Option<Hit> {
    let hit = |summary| Hit {
        rule: "dns_tunnel",
        severity: "medium",
        summary,
        evidence: None,
    };
    if let Some(name) = names
        .iter()
        .find(|name| name.name.split('.').any(|label| label.len() >= LABEL))
    {
        return Some(hit(format!(
            "Asked DNS for {}, a name with a label long enough to carry data",
            short(&name.name)
        )));
    }
    let mut domains: HashMap<String, usize> = HashMap::new();
    for name in names {
        let labels: Vec<&str> = name.name.rsplitn(3, '.').collect();
        if labels.len() == 3 {
            *domains
                .entry(format!("{}.{}", labels[1], labels[0]))
                .or_default() += 1;
        }
    }
    let (domain, count) = domains
        .into_iter()
        .filter(|(_, count)| *count >= NAMES_PER_DOMAIN)
        .max_by_key(|(_, count)| *count)?;
    Some(hit(format!(
        "Asked DNS for {count} different names under {domain} in {minutes} minutes"
    )))
}

/// Reads one agent's traffic from its VMM; `None` when it has no sandbox to read.
pub type Probe =
    Arc<dyn Fn(AgentId) -> Pin<Box<dyn Future<Output = Option<Traffic>> + Send>> + Send + Sync>;

pub fn probe<S: Sandbox + Send + Sync + 'static>(sandbox: Arc<S>) -> Probe {
    Arc::new(move |agent| {
        let sandbox = sandbox.clone();
        Box::pin(async move { read(&*sandbox, agent).await })
    })
}

async fn read<S: Sandbox>(sandbox: &S, agent: AgentId) -> Option<Traffic> {
    let opened = sandbox.exec_channel(agent, &["cubebot-sandbox", "network"]);
    let ExecChannel {
        stdin,
        stdout,
        _session,
    } = tokio::time::timeout(READ_TIMEOUT, opened)
        .await
        .ok()?
        .ok()?;
    drop(stdin);
    let mut bytes = Vec::new();
    let mut limited = stdout.take(READ_LIMIT);
    tokio::time::timeout(READ_TIMEOUT, limited.read_to_end(&mut bytes))
        .await
        .ok()?
        .ok()?;
    serde_json::from_slice(&bytes)
        .inspect_err(|error| tracing::warn!(%error, agent = %agent.0, "unreadable traffic summary"))
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use cubebot_protocol::watch::{Endpoint, Process};

    fn rules(hits: &[Hit]) -> Vec<&str> {
        hits.iter().map(|hit| hit.rule).collect()
    }

    #[test]
    fn sightings_become_findings_and_blocked_addresses_add_up_to_a_scan() {
        let mut sensed = Sensed::default();
        let process = Process {
            pid: 9,
            executable: "/usr/bin/su".into(),
            command: "su -".into(),
        };
        let hits = sensed.report(&[
            Sighting::Escalated(process),
            Sighting::Tampered {
                path: "/etc/passwd".into(),
            },
        ]);
        assert_eq!(rules(&hits), ["privilege_escalation", "tampered"]);
        let blocked = |address: &str| Sighting::Blocked {
            address: address.into(),
            port: 22,
        };
        let hits = sensed.report(&[
            blocked("10.0.0.1"),
            blocked("10.0.0.1"),
            blocked("10.0.0.2"),
        ]);
        assert_eq!(rules(&hits), ["blocked_private"; 3]);
        let hits = sensed.report(&[blocked("10.0.0.3"), blocked("10.0.0.4")]);
        assert_eq!(
            rules(&hits),
            ["lan_scan", "blocked_private"],
            "a scan is reported once"
        );
        assert!(sensed.silence().is_none(), "it just spoke");
        assert!(
            Sensed::default().silence().is_none(),
            "a sensor that never spoke is not silent"
        );
    }

    fn flow(address: &str, sent: u64) -> Flow {
        Flow {
            address: address.into(),
            port: 443,
            protocol: "tcp".into(),
            sent,
            received: 0,
            opened: 1,
        }
    }

    #[test]
    fn traffic_rules_look_at_public_destinations_only() {
        let quiet = Traffic {
            seconds: 300,
            flows: vec![flow("10.43.0.1", 900 * MIB), flow("140.82.112.3", 20 * MIB)],
            ..Traffic::default()
        };
        assert!(
            traffic(&quiet, true).is_empty(),
            "the orchestrator and backups are private"
        );
        let busy = Traffic {
            seconds: 300,
            flows: vec![flow("140.82.112.3", 120 * MIB)],
            unresolved: vec![Endpoint {
                address: "1.2.3.4".into(),
                port: 4444,
            }],
            names: (0..150)
                .map(|n| Name {
                    name: format!("x{n}.tunnel.example"),
                    count: 1,
                })
                .collect(),
            overflow: 0,
        };
        let hits = traffic(&busy, true);
        assert_eq!(
            rules(&hits),
            ["large_upload", "idle_traffic", "dns_tunnel", "direct_ip"]
        );
        assert_eq!(
            hits[0].summary,
            "Sent 120 MB to 140.82.112.3:443 in 5 minutes"
        );
        assert!(
            hits[2]
                .summary
                .contains("150 different names under tunnel.example")
        );
        assert_eq!(
            rules(&traffic(&busy, false))[1],
            "dns_tunnel",
            "working agents may upload"
        );
        let long = vec![Name {
            name: format!("{}.evil.example", "a".repeat(60)),
            count: 1,
        }];
        assert_eq!(
            rules(&tunnel(&long, 5).into_iter().collect::<Vec<_>>()),
            ["dns_tunnel"]
        );
    }
}
