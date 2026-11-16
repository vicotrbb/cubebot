use cubebot_protocol::runtime::{Throughput, Usage, Vitals};
use std::{fs, path::Path, time::Instant};

const CGROUP: &str = "/sys/fs/cgroup";

#[derive(Clone, Copy)]
struct Counters {
    at: Instant,
    cpu: Option<(u64, u64)>,
    network: Option<(u64, u64)>,
}

pub struct Sampler {
    started: Instant,
    cpus: u32,
    last: Option<Counters>,
}

impl Default for Sampler {
    fn default() -> Self {
        let cpus = std::thread::available_parallelism().map_or(1, |cpus| cpus.get());
        Self {
            started: Instant::now(),
            cpus: u32::try_from(cpus).unwrap_or(u32::MAX),
            last: None,
        }
    }
}

impl Sampler {
    pub fn sample(&mut self, workspace: &Path) -> Vitals {
        let now = Counters {
            at: Instant::now(),
            cpu: self.cpu(),
            network: read("/proc/net/dev").and_then(|text| network(&text)),
        };
        let last = self.last.replace(now);
        let seconds = last.map(|last| now.at.duration_since(last.at).as_secs_f64());
        Vitals {
            cpu_percent: last.and_then(|last| busy(last.cpu?, now.cpu?)),
            cpus: self.cpus,
            memory: memory(),
            disk: disk(workspace),
            network: last
                .zip(seconds.filter(|seconds| *seconds > 0.0))
                .and_then(|(last, seconds)| rate(last.network?, now.network?, seconds)),
        }
    }

    fn cpu(&self) -> Option<(u64, u64)> {
        let wall = u64::try_from(self.started.elapsed().as_micros()).ok()?;
        match read(&format!("{CGROUP}/cpu.stat")).and_then(|text| field(&text, "usage_usec")) {
            Some(usage) => Some((usage, wall.saturating_mul(self.cpus.into()))),
            None => read("/proc/stat").and_then(|text| processor(&text)),
        }
    }
}

fn read(path: &str) -> Option<String> {
    fs::read_to_string(path).ok()
}

fn field(text: &str, name: &str) -> Option<u64> {
    text.lines().find_map(|line| {
        let (key, value) = line.split_once([' ', ':'])?;
        (key == name).then(|| value.trim().split(' ').next()?.parse().ok())?
    })
}

fn processor(stat: &str) -> Option<(u64, u64)> {
    let line = stat.lines().find(|line| line.starts_with("cpu "))?;
    let ticks: Vec<u64> = line
        .split_whitespace()
        .skip(1)
        .take(8)
        .map_while(|tick| tick.parse().ok())
        .collect();
    let total: u64 = ticks.iter().sum();
    let idle = ticks.get(3)? + ticks.get(4).copied().unwrap_or(0);
    Some((total - idle, total))
}

fn busy(last: (u64, u64), now: (u64, u64)) -> Option<f64> {
    let total = now.1.checked_sub(last.1).filter(|total| *total > 0)?;
    let busy = now.0.saturating_sub(last.0);
    Some((busy as f64 / total as f64 * 100.0).clamp(0.0, 100.0))
}

fn network(dev: &str) -> Option<(u64, u64)> {
    let mut sum = (0, 0);
    for line in dev.lines().skip(2) {
        let (interface, counters) = line.split_once(':')?;
        if interface.trim() == "lo" {
            continue;
        }
        let counters: Vec<u64> = counters
            .split_whitespace()
            .map_while(|counter| counter.parse().ok())
            .collect();
        sum.0 += counters.first()?;
        sum.1 += counters.get(8)?;
    }
    Some(sum)
}

fn rate(last: (u64, u64), now: (u64, u64), seconds: f64) -> Option<Throughput> {
    let per_second = |from: u64, to: u64| (to.saturating_sub(from) as f64 / seconds) as u64;
    Some(Throughput {
        received: per_second(last.0, now.0),
        sent: per_second(last.1, now.1),
    })
}

fn memory() -> Option<Usage> {
    let meminfo = read("/proc/meminfo")?;
    let kibibytes = |name| field(&meminfo, name).map(|value| value * 1024);
    let installed = kibibytes("MemTotal")?;
    let limit = read(&format!("{CGROUP}/memory.max")).and_then(|max| max.trim().parse().ok());
    match limit {
        Some(limit) => {
            let current: u64 = read(&format!("{CGROUP}/memory.current"))?
                .trim()
                .parse()
                .ok()?;
            let stat = read(&format!("{CGROUP}/memory.stat")).unwrap_or_default();
            let inactive = field(&stat, "inactive_file").unwrap_or(0);
            Some(Usage {
                used: current.saturating_sub(inactive),
                total: installed.min(limit),
            })
        }
        None => Some(Usage {
            used: installed.saturating_sub(kibibytes("MemAvailable")?),
            total: installed,
        }),
    }
}

fn disk(workspace: &Path) -> Option<Usage> {
    let stat = nix::sys::statvfs::statvfs(workspace).ok()?;
    let total = bytes(stat.blocks(), stat.fragment_size());
    let free = bytes(stat.blocks_free(), stat.fragment_size());
    Some(Usage {
        used: total.saturating_sub(free),
        total,
    })
}

fn bytes(blocks: impl Into<u64>, size: impl Into<u64>) -> u64 {
    blocks.into().saturating_mul(size.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn processor_time_counts_everything_but_idle_and_waiting() {
        let stat = "cpu  100 0 50 800 50 0 0 0 0 0\ncpu0 100 0 50 800 50 0 0 0 0 0\n";
        assert_eq!(processor(stat), Some((150, 1000)));
        assert_eq!(busy((150, 1000), (450, 2000)), Some(30.0));
        assert_eq!(busy((150, 1000), (150, 1000)), None);
    }

    #[test]
    fn cgroup_and_meminfo_fields_read_by_name() {
        let stat = "usage_usec 4200\nuser_usec 4000\nsystem_usec 200\n";
        assert_eq!(field(stat, "usage_usec"), Some(4200));
        let meminfo =
            "MemTotal:        2048000 kB\nMemFree:          100 kB\nMemAvailable:    1024000 kB\n";
        assert_eq!(field(meminfo, "MemAvailable"), Some(1_024_000));
        assert_eq!(field(meminfo, "Missing"), None);
    }

    #[test]
    fn network_sums_every_interface_but_loopback() {
        let dev = "Inter-|   Receive                                                |  Transmit\n face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed\n    lo: 999 1 0 0 0 0 0 0 999 1 0 0 0 0 0 0\n  eth0: 1000 10 0 0 0 0 0 0 400 4 0 0 0 0 0 0\n  tap1: 24 1 0 0 0 0 0 0 600 2 0 0 0 0 0 0\n";
        assert_eq!(network(dev), Some((1024, 1000)));
        assert_eq!(
            rate((24, 0), (1024, 1000), 2.0),
            Some(Throughput {
                received: 500,
                sent: 500
            })
        );
    }

    #[test]
    fn a_first_sample_has_no_rates_yet() {
        let mut sampler = Sampler::default();
        let first = sampler.sample(Path::new("/"));
        assert!(first.cpu_percent.is_none() && first.network.is_none());
        assert!(first.cpus >= 1);
        assert!(first.disk.is_some_and(|disk| disk.total >= disk.used));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_second_linux_sample_measures_every_vital() {
        let mut sampler = Sampler::default();
        sampler.sample(Path::new("/"));
        std::thread::sleep(std::time::Duration::from_millis(300));
        let second = sampler.sample(Path::new("/"));
        assert!(
            second
                .cpu_percent
                .is_some_and(|cpu| (0.0..=100.0).contains(&cpu))
        );
        assert!(
            second
                .memory
                .is_some_and(|memory| memory.total >= memory.used && memory.used > 0)
        );
        assert!(second.network.is_some());
    }
}
