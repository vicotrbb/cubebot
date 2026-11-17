mod channel;
mod server;
mod supervisor;

pub use server::serve;

use crate::{
    connections::Connections,
    control::{MemoryClient, RuntimeClient},
    tools::Sandbox,
};
use cubebot_protocol::apps::{AppReport, AppSpec};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, RwLock},
    time::Duration,
};
use tokio::sync::mpsc;

const REFRESH: Duration = Duration::from_secs(60);
const QUEUED_REPORTS: usize = 256;

pub struct Apps {
    specs: RwLock<Vec<AppSpec>>,
    reports: mpsc::Sender<AppReport>,
    unclaimed: Mutex<Option<mpsc::Receiver<AppReport>>>,
    client: Option<MemoryClient>,
    connections: Connections,
    sandbox: Option<Sandbox>,
    supervised: tokio::sync::Mutex<HashMap<String, supervisor::Process>>,
}

impl Apps {
    pub fn new(client: Option<MemoryClient>, sandbox: Option<Sandbox>) -> Self {
        let (reports, unclaimed) = mpsc::channel(QUEUED_REPORTS);
        Self {
            specs: RwLock::default(),
            reports,
            unclaimed: Mutex::new(Some(unclaimed)),
            connections: Connections::new(client.clone(), &[]),
            client,
            sandbox,
            supervised: tokio::sync::Mutex::default(),
        }
    }

    pub fn reports(&self) -> Option<mpsc::Receiver<AppReport>> {
        self.unclaimed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    pub async fn refresh(&self) {
        let Some(client) = &self.client else {
            return;
        };
        let mut supervised = self.supervised.lock().await;
        let specs = match client.apps().await {
            Ok(specs) => specs,
            Err(error) => {
                eprintln!("Apps not refreshed: {error}");
                return;
            }
        };
        supervised.retain(|slug, _| specs.iter().any(|spec| &spec.slug == slug));
        for spec in &specs {
            if supervised
                .get(&spec.slug)
                .is_none_or(|process| process.spec != *spec)
            {
                let process = supervisor::Process::start(spec, &self.sandbox, &self.reports);
                supervised.insert(spec.slug.clone(), process);
            }
        }
        *self
            .specs
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = specs;
    }

    fn spec(&self, slug: &str) -> Option<AppSpec> {
        let specs = self
            .specs
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        specs.iter().find(|spec| spec.slug == slug).cloned()
    }
}

fn report(reports: &mpsc::Sender<AppReport>, report: AppReport) {
    let _ = reports.try_send(report);
}

pub async fn run(apps: Arc<Apps>, client: RuntimeClient) {
    tokio::spawn(serve(apps.clone()));
    tokio::spawn(channel::run(apps.clone(), client));
    let mut every = tokio::time::interval(REFRESH);
    loop {
        every.tick().await;
        apps.refresh().await;
    }
}
