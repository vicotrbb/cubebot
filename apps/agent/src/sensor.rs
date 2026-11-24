//! The sandbox sensor (docs/adr/agents.md#a-watchdog-that-notifies). With each vitals sample it
//! reads `/proc` for processes and TCP sockets, and every minute it looks at a few files only root
//! can change and the places tools write; each new sighting is reported once. It runs in the
//! agent process, so it sees what the tool and browser users run. A guest taken over as root can
//! blind it, which the orchestrator notices as its silence.
use cubebot_protocol::watch::{Process, Sighting};
use std::{
    collections::{HashMap, HashSet},
    fs,
    net::Ipv4Addr,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const MINERS: &[&str] = &[
    "xmrig",
    "minerd",
    "cpuminer",
    "ethminer",
    "nbminer",
    "lolminer",
    "stratum+tcp://",
    "stratum+ssl://",
    "stratum2+tcp://",
];
const TEMPORARY: &[&str] = &["/tmp/", "/var/tmp/", "/dev/shm/"];
/// Build tools that run what they just compiled from a temporary directory.
const BUILDS: &[&str] = &[
    "/go-build",
    "/cargo-install",
    "/pip-",
    "/node-gyp",
    "/rustc",
];
/// Mail sent straight to servers, mining pools, common remote-shell ports and Tor.
const ODD_PORTS: &[u16] = &[25, 3333, 4444, 9050, 9150, 14444, 45700];
const SENTINELS: &[&str] = &[
    "/etc/passwd",
    "/etc/shadow",
    "/etc/group",
    "/etc/sudoers",
    "/etc/ld.so.preload",
];
const FLOOD: usize = 1000;
const FILES: Duration = Duration::from_secs(60);
/// A connection sighting is reported again only after this long.
const REMEMBER: Duration = Duration::from_secs(5 * 60);
/// Entries a setuid scan reads, so a huge workspace never slows a sample.
const SCAN_LIMIT: usize = 5000;

#[derive(Clone, Debug, PartialEq)]
struct Entry {
    pid: u32,
    parent: u32,
    real: u32,
    effective: u32,
    started: u64,
    executable: String,
    command: String,
}

/// Inode, size, modification and change times: a write or a replacement changes the last.
type Fingerprint = Option<(u64, u64, i64, i64)>;

pub struct Sensor {
    proc: PathBuf,
    places: Vec<PathBuf>,
    reported: HashSet<(u32, u64)>,
    unanswered: HashSet<(Ipv4Addr, u16)>,
    recent: HashMap<(Ipv4Addr, u16), Instant>,
    flooded: bool,
    sentinels: HashMap<PathBuf, Fingerprint>,
    setuid: HashSet<PathBuf>,
    checked: Instant,
}

impl Sensor {
    pub fn new(workspace: &Path) -> Self {
        let mut watched: Vec<PathBuf> = SENTINELS.iter().map(PathBuf::from).collect();
        watched.extend(std::env::current_exe().ok());
        let places = ["/tmp", "/var/tmp", "/dev/shm"]
            .into_iter()
            .map(PathBuf::from)
            .chain([workspace.to_owned()])
            .collect::<Vec<_>>();
        Self {
            proc: PathBuf::from("/proc"),
            sentinels: watched
                .into_iter()
                .map(|path| {
                    let print = fingerprint(&path);
                    (path, print)
                })
                .collect(),
            setuid: setuid_files(&places),
            places,
            reported: HashSet::new(),
            unanswered: HashSet::new(),
            recent: HashMap::new(),
            flooded: false,
            checked: Instant::now(),
        }
    }

    pub fn sample(&mut self) -> Vec<Sighting> {
        let table = processes(&self.proc);
        let mut seen = self.judge(&table);
        seen.extend(self.sockets());
        if self.checked.elapsed() >= FILES {
            self.checked = Instant::now();
            seen.extend(self.files());
        }
        seen
    }

    fn judge(&mut self, table: &[Entry]) -> Vec<Sighting> {
        let parents: HashMap<u32, &Entry> = table.iter().map(|entry| (entry.pid, entry)).collect();
        let mut seen: Vec<Sighting> = table
            .iter()
            .filter(|entry| !self.reported.contains(&(entry.pid, entry.started)))
            .filter_map(|entry| classify(entry, &parents))
            .collect();
        for sighting in &seen {
            if let Some(process) = process_of(sighting) {
                let started = parents.get(&process.pid).map_or(0, |entry| entry.started);
                self.reported.insert((process.pid, started));
            }
        }
        let live: HashSet<(u32, u64)> = table.iter().map(|e| (e.pid, e.started)).collect();
        self.reported.retain(|key| live.contains(key));
        let tools = table.iter().filter(|entry| entry.effective != 0).count();
        if tools >= FLOOD && !self.flooded {
            seen.push(Sighting::Flood {
                processes: u32::try_from(tools).unwrap_or(u32::MAX),
            });
        }
        self.flooded = tools >= FLOOD || (self.flooded && tools >= FLOOD / 2);
        seen
    }

    fn sockets(&mut self) -> Vec<Sighting> {
        let sockets: Vec<Socket> = ["net/tcp", "net/tcp6"]
            .iter()
            .filter_map(|table| fs::read_to_string(self.proc.join(table)).ok())
            .flat_map(|text| text.lines().skip(1).filter_map(socket).collect::<Vec<_>>())
            .filter(|socket| socket.uid != 0)
            .collect();
        self.recent.retain(|_, at| at.elapsed() < REMEMBER);
        let waiting: HashSet<(Ipv4Addr, u16)> = sockets
            .iter()
            .filter(|socket| socket.state == SYN_SENT && private(socket.remote))
            .map(|socket| (socket.remote, socket.port))
            .collect();
        let mut seen = Vec::new();
        for &(address, port) in waiting.intersection(&self.unanswered) {
            if self
                .recent
                .insert((address, port), Instant::now())
                .is_none()
            {
                seen.push(Sighting::Blocked {
                    address: address.to_string(),
                    port,
                });
            }
        }
        self.unanswered = waiting;
        for socket in sockets
            .iter()
            .filter(|socket| ODD_PORTS.contains(&socket.port))
        {
            let key = (socket.remote, socket.port);
            if matches!(socket.state, SYN_SENT | ESTABLISHED)
                && self.recent.insert(key, Instant::now()).is_none()
            {
                seen.push(Sighting::OddPort {
                    address: socket.remote.to_string(),
                    port: socket.port,
                });
            }
        }
        seen
    }

    fn files(&mut self) -> Vec<Sighting> {
        let mut seen = Vec::new();
        for (path, known) in &mut self.sentinels {
            let now = fingerprint(path);
            if now != *known {
                *known = now;
                seen.push(Sighting::Tampered {
                    path: path.display().to_string(),
                });
            }
        }
        for path in setuid_files(&self.places) {
            if self.setuid.insert(path.clone()) {
                seen.push(Sighting::Setuid {
                    path: path.display().to_string(),
                });
            }
        }
        seen
    }
}

fn classify(entry: &Entry, table: &HashMap<u32, &Entry>) -> Option<Sighting> {
    let process = || Process {
        pid: entry.pid,
        executable: entry.executable.clone(),
        command: entry.command.clone(),
    };
    if entry.effective == 0 {
        return escalated(entry, table).then(|| Sighting::Escalated(process()));
    }
    let command = entry.command.to_lowercase();
    if MINERS.iter().any(|miner| command.contains(miner)) {
        return Some(Sighting::Miner(process()));
    }
    hidden(&entry.executable).then(|| Sighting::Hidden(process()))
}

/// Root with a real user that is not root, or under a process that is not: what Cubebot starts as
/// root has root ancestors all the way up.
fn escalated(entry: &Entry, table: &HashMap<u32, &Entry>) -> bool {
    if entry.pid <= 2 {
        return false;
    }
    if entry.real != 0 {
        return true;
    }
    let mut parent = entry.parent;
    for _ in 0..64 {
        let Some(ancestor) = table.get(&parent) else {
            return false;
        };
        if ancestor.effective != 0 {
            return true;
        }
        parent = ancestor.parent;
    }
    false
}

fn hidden(executable: &str) -> bool {
    let temporary = TEMPORARY.iter().any(|place| executable.starts_with(place))
        && !BUILDS.iter().any(|build| executable.contains(build));
    temporary || executable.ends_with(" (deleted)")
}

fn process_of(sighting: &Sighting) -> Option<&Process> {
    match sighting {
        Sighting::Escalated(process) | Sighting::Miner(process) | Sighting::Hidden(process) => {
            Some(process)
        }
        _ => None,
    }
}

fn processes(proc: &Path) -> Vec<Entry> {
    let Ok(entries) = fs::read_dir(proc) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|entry| entry.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| read_process(&proc.join(pid.to_string()), pid))
        .collect()
}

fn read_process(directory: &Path, pid: u32) -> Option<Entry> {
    let status = fs::read_to_string(directory.join("status")).ok()?;
    let stat = fs::read_to_string(directory.join("stat")).ok()?;
    let command = fs::read(directory.join("cmdline")).unwrap_or_default();
    let command = String::from_utf8_lossy(&command).replace('\0', " ");
    let command = command.trim().chars().take(300).collect::<String>();
    // Another user's exe link needs ptrace rights the container runtime does not have.
    let executable = fs::read_link(directory.join("exe")).map_or_else(
        |_| command.split(' ').next().unwrap_or_default().to_owned(),
        |path| path.display().to_string(),
    );
    let (parent, real, effective) = ids(&status)?;
    Some(Entry {
        pid,
        parent,
        real,
        effective,
        started: started(&stat)?,
        executable,
        command,
    })
}

/// The parent and the real and effective user from `/proc/<pid>/status`.
fn ids(status: &str) -> Option<(u32, u32, u32)> {
    let field = |name: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(name))
            .map(|rest| rest.split_whitespace().collect::<Vec<_>>())
    };
    let parent = field("PPid:")?.first()?.parse().ok()?;
    let users = field("Uid:")?;
    Some((
        parent,
        users.first()?.parse().ok()?,
        users.get(1)?.parse().ok()?,
    ))
}

/// The start time from `/proc/<pid>/stat`, read after the command name, which may hold spaces.
fn started(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    rest.split_whitespace().nth(19)?.parse().ok()
}

const ESTABLISHED: u8 = 0x01;
const SYN_SENT: u8 = 0x02;

#[derive(Debug, PartialEq)]
struct Socket {
    remote: Ipv4Addr,
    port: u16,
    state: u8,
    uid: u32,
}

/// One line of `/proc/net/tcp` or `tcp6`; IPv6 only when it carries an IPv4 address.
fn socket(line: &str) -> Option<Socket> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    let (address, port) = fields.get(2)?.split_once(':')?;
    Some(Socket {
        remote: ipv4(address)?,
        port: u16::from_str_radix(port, 16).ok()?,
        state: u8::from_str_radix(fields.get(3)?, 16).ok()?,
        uid: fields.get(7)?.parse().ok()?,
    })
}

/// The kernel prints each 32-bit word in host order, little-endian on the guests Cubebot runs.
fn ipv4(hex: &str) -> Option<Ipv4Addr> {
    let word = |at: usize| u32::from_str_radix(hex.get(at..at + 8)?, 16).ok();
    match hex.len() {
        8 => Some(Ipv4Addr::from(word(0)?.to_le_bytes())),
        32 if word(0)? == 0 && word(8)? == 0 && word(16)? == 0xffff_0000 => {
            Some(Ipv4Addr::from(word(24)?.to_le_bytes()))
        }
        _ => None,
    }
}

fn private(address: Ipv4Addr) -> bool {
    let [first, second, ..] = address.octets();
    address.is_private() || address.is_link_local() || (first == 100 && (64..128).contains(&second))
}

fn fingerprint(path: &Path) -> Fingerprint {
    fs::metadata(path)
        .ok()
        .map(|meta| (meta.ino(), meta.size(), meta.mtime(), meta.ctime()))
}

/// Root-owned setuid or setgid files under these places, symlinks not followed.
fn setuid_files(places: &[PathBuf]) -> HashSet<PathBuf> {
    let mut found = HashSet::new();
    let mut stack: Vec<(PathBuf, u8)> = places.iter().map(|place| (place.clone(), 0)).collect();
    let mut read = 0;
    while let Some((directory, depth)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            read += 1;
            if read > SCAN_LIMIT {
                return found;
            }
            let Ok(meta) = entry.path().symlink_metadata() else {
                continue;
            };
            if meta.is_dir() && depth < 6 {
                stack.push((entry.path(), depth + 1));
            } else if meta.is_file() && meta.uid() == 0 && meta.mode() & 0o6000 != 0 {
                found.insert(entry.path());
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(pid: u32, parent: u32, real: u32, effective: u32, executable: &str) -> Entry {
        Entry {
            pid,
            parent,
            real,
            effective,
            started: u64::from(pid),
            executable: executable.into(),
            command: executable.into(),
        }
    }

    fn sensor() -> Sensor {
        Sensor {
            proc: PathBuf::from("/nonexistent"),
            places: Vec::new(),
            reported: HashSet::new(),
            unanswered: HashSet::new(),
            recent: HashMap::new(),
            flooded: false,
            sentinels: HashMap::new(),
            setuid: HashSet::new(),
            checked: Instant::now(),
        }
    }

    #[test]
    fn root_under_a_tool_is_escalation_and_is_reported_once() {
        let table = vec![
            entry(1, 0, 0, 0, "/usr/local/bin/cubebot-agent"),
            entry(2, 0, 0, 0, ""),
            entry(40, 1, 0, 0, "/usr/bin/Xephyr"),
            entry(50, 1, 10001, 10001, "/bin/bash"),
            entry(60, 50, 0, 0, "/tmp/x"),
            entry(61, 50, 10001, 0, "/usr/bin/su"),
            entry(62, 40, 0, 0, "/usr/bin/openbox"),
        ];
        let mut sensor = sensor();
        let seen = sensor.judge(&table);
        let pids: Vec<u32> = seen.iter().filter_map(process_of).map(|p| p.pid).collect();
        assert_eq!(pids, [60, 61]);
        assert!(matches!(seen[0], Sighting::Escalated(_)));
        assert!(
            sensor.judge(&table).is_empty(),
            "each process is reported once"
        );
    }

    #[test]
    fn miners_and_hidden_executables_are_seen_in_tool_processes_only() {
        let mut miner = entry(70, 1, 10001, 10001, "/home/tool/run");
        miner.command = "./run -o stratum+tcp://pool:3333".into();
        let table = vec![
            miner,
            entry(71, 1, 10001, 10001, "/tmp/payload"),
            entry(72, 1, 10001, 10001, "/tmp/go-build123/b001/exe/main"),
            entry(73, 1, 10001, 10001, "/home/tool/app (deleted)"),
            entry(74, 1, 0, 0, "/tmp/root-owned-by-design"),
        ];
        let seen = sensor().judge(&table);
        let kinds: Vec<_> = seen
            .iter()
            .map(|sighting| match sighting {
                Sighting::Miner(process) => ("miner", process.pid),
                Sighting::Hidden(process) => ("hidden", process.pid),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(kinds, [("miner", 70), ("hidden", 71), ("hidden", 73)]);
    }

    #[test]
    fn a_flood_is_reported_once_until_it_subsides() {
        let mut sensor = sensor();
        let crowd: Vec<Entry> = (10..10 + FLOOD as u32)
            .map(|pid| entry(pid, 1, 10001, 10001, "/bin/sleep"))
            .collect();
        let floods = |seen: Vec<Sighting>| {
            seen.into_iter()
                .filter(|s| matches!(s, Sighting::Flood { .. }))
                .count()
        };
        assert_eq!(floods(sensor.judge(&crowd)), 1);
        assert_eq!(floods(sensor.judge(&crowd)), 0);
        assert_eq!(floods(sensor.judge(&crowd[..10])), 0);
        assert_eq!(floods(sensor.judge(&crowd)), 1);
    }

    #[test]
    fn proc_text_is_parsed() {
        let status = "Name:\tbash\nPPid:\t50\nUid:\t10001\t0\t0\t0\n";
        assert_eq!(ids(status), Some((50, 10001, 0)));
        let stat = "60 (a (weird) name) S 50 60 50 0 -1 4194560 100 0 0 0 0 0 0 0 20 0 1 0 12345 0";
        assert_eq!(started(stat), Some(12345));
        let tcp = "   1: 0200000A:A2B4 0101FF0A:0050 02 00000000:00000000 01:00000000 00000000 10001        0 12345";
        assert_eq!(
            socket(tcp),
            Some(Socket {
                remote: Ipv4Addr::new(10, 255, 1, 1),
                port: 80,
                state: SYN_SENT,
                uid: 10001
            })
        );
        let mapped = "0000000000000000FFFF00000101FF0A";
        assert_eq!(ipv4(mapped), Some(Ipv4Addr::new(10, 255, 1, 1)));
        assert_eq!(ipv4("20010DB8000000000000000000000001"), None);
        assert!(
            private(Ipv4Addr::new(100, 64, 0, 1)) && private(Ipv4Addr::new(169, 254, 169, 254))
        );
        assert!(!private(Ipv4Addr::new(8, 8, 8, 8)) && !private(Ipv4Addr::new(127, 0, 0, 1)));
    }

    #[test]
    fn sightings_keep_their_wire_shape() {
        let sighting = Sighting::Escalated(Process {
            pid: 60,
            executable: "/tmp/x".into(),
            command: "/tmp/x".into(),
        });
        let text = serde_json::to_string(&sighting).unwrap();
        assert_eq!(
            text,
            r#"{"kind":"escalated","pid":60,"executable":"/tmp/x","command":"/tmp/x"}"#
        );
        assert_eq!(serde_json::from_str::<Sighting>(&text).unwrap(), sighting);
    }
}
