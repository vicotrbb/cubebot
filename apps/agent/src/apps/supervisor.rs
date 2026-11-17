use super::report;
use crate::tools::{Group, Sandbox};
use cubebot_protocol::apps::{AppReport, AppSpec, Desired, Reported};
use std::{collections::VecDeque, process::Stdio, time::Duration};
use tokio::{net::TcpStream, sync::mpsc, task::JoinHandle, time::Instant};

#[cfg(not(test))]
const BACKOFF: Duration = Duration::from_secs(1);
#[cfg(test)]
const BACKOFF: Duration = Duration::from_millis(10);
const BACKOFF_LIMIT: u32 = 30;
const ATTEMPTS: usize = 5;
const WINDOW: Duration = Duration::from_secs(120);
const STATUS: Duration = Duration::from_secs(10);
const READY_POLL: Duration = Duration::from_millis(250);
const LOG_LIMIT: u64 = 2 * 1024 * 1024;
const LOGS: &str = ".cubebot-apps";

const LAUNCH: &str = r#"mkdir -p .cubebot-apps && exec >> ".cubebot-apps/$2.log" 2>&1 && cd -- "$3" && exec /bin/sh -c "$1""#;
const TRIM: &str = r#"log=".cubebot-apps/$1.log" && tail -c 1048576 "$log" > "$log.tmp" && cat "$log.tmp" > "$log"; rm -f "$log.tmp""#;

pub(super) struct Process {
    pub(super) spec: AppSpec,
    task: Option<JoinHandle<()>>,
}

impl Drop for Process {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}

impl Process {
    pub(super) fn start(
        spec: &AppSpec,
        sandbox: &Option<Sandbox>,
        reports: &mpsc::Sender<AppReport>,
    ) -> Self {
        let launch = match (spec.desired, &spec.command, spec.port, sandbox) {
            (Desired::Stopped, ..) => Err(Reported::Stopped),
            (Desired::Running, None, _, Some(_)) => Err(Reported::Running),
            (Desired::Running, Some(command), Some(port), Some(sandbox)) => Ok(Run {
                slug: spec.slug.clone(),
                folder: spec.folder.clone(),
                command: command.clone(),
                port,
                sandbox: sandbox.clone(),
                reports: reports.clone(),
            }),
            _ => Err(Reported::Failed),
        };
        let task = match launch {
            Ok(run) => Some(tokio::spawn(run.supervise())),
            Err(state) => {
                let slug = spec.slug.clone();
                report(
                    reports,
                    AppReport::State {
                        slug,
                        state,
                        memory: None,
                    },
                );
                None
            }
        };
        Self {
            spec: spec.clone(),
            task,
        }
    }
}

struct Run {
    slug: String,
    folder: String,
    command: String,
    port: u16,
    sandbox: Sandbox,
    reports: mpsc::Sender<AppReport>,
}

impl Run {
    async fn supervise(self) {
        let mut exits = VecDeque::new();
        loop {
            self.report(Reported::Starting, None);
            if !exits.is_empty() {
                tokio::time::sleep(backoff(exits.len())).await;
            }
            self.once().await;
            let now = Instant::now();
            exits.retain(|at| now.duration_since(*at) < WINDOW);
            exits.push_back(now);
            if exits.len() >= ATTEMPTS {
                self.report(Reported::Failed, None);
                return;
            }
        }
    }

    async fn once(&self) {
        self.trim().await;
        let mut command = self.sandbox.command("/bin/sh");
        command
            .args(["-c", LAUNCH, "app", &self.command, &self.slug, &self.folder])
            .env("PORT", self.port.to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        let mut child = match command.spawn() {
            Ok(child) => child,
            Err(error) => {
                eprintln!("App {} did not start: {error}", self.slug);
                return;
            }
        };
        let group = child.id();
        let _group = Group(group);
        let ready = ready(self.port);
        tokio::pin!(ready);
        let mut state = Reported::Starting;
        let mut status = tokio::time::interval_at(Instant::now() + STATUS, STATUS);
        loop {
            tokio::select! {
                _ = child.wait() => return,
                _ = &mut ready, if state == Reported::Starting => {
                    state = Reported::Running;
                    self.report(state, sample(group).await);
                }
                _ = status.tick() => {
                    self.report(state, sample(group).await);
                    self.trim().await;
                }
            }
        }
    }

    async fn trim(&self) {
        let log = self
            .sandbox
            .workspace()
            .join(LOGS)
            .join(format!("{}.log", self.slug));
        let large = std::fs::symlink_metadata(log).is_ok_and(|log| log.len() > LOG_LIMIT);
        if !large {
            return;
        }
        let mut command = self.sandbox.command("/bin/sh");
        command
            .args(["-c", TRIM, "trim", &self.slug])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if let Err(error) = command.status().await {
            eprintln!("App {} log not trimmed: {error}", self.slug);
        }
    }

    fn report(&self, state: Reported, memory: Option<u64>) {
        let slug = self.slug.clone();
        report(
            &self.reports,
            AppReport::State {
                slug,
                state,
                memory,
            },
        );
    }
}

fn backoff(exits: usize) -> Duration {
    let doubled = BACKOFF.saturating_mul(1 << (exits.saturating_sub(1)).min(5));
    doubled.min(BACKOFF * BACKOFF_LIMIT)
}

async fn ready(port: u16) {
    while TcpStream::connect((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .is_err()
    {
        tokio::time::sleep(READY_POLL).await;
    }
}

async fn sample(group: Option<u32>) -> Option<u64> {
    let group = group?;
    tokio::task::spawn_blocking(move || memory(group))
        .await
        .ok()
        .flatten()
}

fn memory(group: u32) -> Option<u64> {
    let mut total = None;
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let path = entry.path();
        let in_group = std::fs::read_to_string(path.join("stat"))
            .ok()
            .and_then(|stat| process_group(&stat))
            == Some(group);
        if let Some(rss) = in_group
            .then(|| std::fs::read_to_string(path.join("status")).ok())
            .flatten()
            .and_then(|status| resident(&status))
        {
            total = Some(total.unwrap_or(0) + rss);
        }
    }
    total
}

fn process_group(stat: &str) -> Option<u32> {
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(2)?
        .parse()
        .ok()
}

fn resident(status: &str) -> Option<u64> {
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kib: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kib * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_crash_loop_backs_off_then_fails() {
        let sandbox = crate::tools::tests::sandbox();
        std::fs::create_dir(sandbox.workspace().join("site")).unwrap();
        let (sender, mut reports) = mpsc::channel(64);
        let spec = AppSpec {
            slug: "crash".into(),
            name: "Crash".into(),
            folder: "site".into(),
            command: Some("echo boom; exit 1".into()),
            port: Some(1),
            tools: Vec::new(),
            desired: Desired::Running,
        };
        let started = std::time::Instant::now();
        let mut process = Process::start(&spec, &Some(sandbox.clone()), &sender);
        drop(sender);
        process.task.take().unwrap().await.unwrap();
        let elapsed = started.elapsed();
        let mut states = Vec::new();
        while let Ok(AppReport::State { state, .. }) = reports.try_recv() {
            states.push(state);
        }
        let mut expected = vec![Reported::Starting; ATTEMPTS];
        expected.push(Reported::Failed);
        assert_eq!(states, expected);
        assert!(elapsed >= BACKOFF * (1 + 2 + 4 + 8), "{elapsed:?}");
        let log = std::fs::read_to_string(sandbox.workspace().join(".cubebot-apps/crash.log"));
        assert_eq!(log.unwrap().matches("boom").count(), ATTEMPTS);
        std::fs::remove_dir_all(sandbox.workspace()).unwrap();
    }

    #[test]
    fn a_static_app_without_a_sandbox_is_not_reported_running() {
        let (sender, mut reports) = mpsc::channel(4);
        let spec = AppSpec {
            slug: "site".into(),
            name: "Site".into(),
            folder: "site".into(),
            command: None,
            port: None,
            tools: Vec::new(),
            desired: Desired::Running,
        };
        let process = Process::start(&spec, &None, &sender);
        assert!(process.task.is_none());
        let reported = reports.try_recv().unwrap();
        let failed = AppReport::State {
            slug: "site".into(),
            state: Reported::Failed,
            memory: None,
        };
        assert_eq!(reported, failed);
        let sandbox = crate::tools::tests::sandbox();
        let process = Process::start(&spec, &Some(sandbox.clone()), &sender);
        let running = AppReport::State {
            slug: "site".into(),
            state: Reported::Running,
            memory: None,
        };
        assert_eq!(reports.try_recv().unwrap(), running);
        drop(process);
        std::fs::remove_dir_all(sandbox.workspace()).unwrap();
    }

    #[test]
    fn group_and_resident_memory_parse_from_proc() {
        let stat = "42 (my (odd) app) S 1 42 42 0 -1";
        assert_eq!(process_group(stat), Some(42));
        assert_eq!(resident("Name:\tx\nVmRSS:\t  2048 kB\n"), Some(2048 * 1024));
    }
}
