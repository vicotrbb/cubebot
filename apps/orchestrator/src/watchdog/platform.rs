//! What the watchdog reads about the platform itself (docs/adr/agents.md#a-watchdog-that-notifies):
//! the pods in its namespace against what Cubebot created, and bursts of refused requests from
//! one session. Nodes stay outside it; a host sensor the operator runs watches those.
use super::rules::Hit;
use k8s_openapi::api::core::v1::Pod;
use serde_json::Value;
use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use uuid::Uuid;

/// Lists the pods of the installation's namespace, with its name; `None` when it cannot.
pub type Pods =
    Arc<dyn Fn() -> Pin<Box<dyn Future<Output = Option<(String, Vec<Pod>)>> + Send>> + Send + Sync>;

/// An agent pod Cubebot recorded creating, with the manifest it created it from.
pub struct Recorded {
    pub agent: Uuid,
    pub manifest: Value,
}

/// What one pod shows: findings with the agent they concern, if any.
pub fn pod(pod: &Pod, recorded: &HashMap<String, Recorded>) -> Vec<(Hit, Option<Uuid>)> {
    let metadata = &pod.metadata;
    if metadata.deletion_timestamp.is_some() {
        return Vec::new();
    }
    let name = metadata.name.clone().unwrap_or_default();
    let labels = metadata.labels.clone().unwrap_or_default();
    let agent_pod = labels.contains_key("cubebot.dev/agent");
    let chart_pod = labels.get("app.kubernetes.io/part-of").map(String::as_str) == Some("cubebot");
    let mut hits = Vec::new();
    let agent = match (agent_pod, recorded.get(&name)) {
        (true, Some(record)) => {
            hits.extend(swapped(pod, record).map(|hit| (hit, Some(record.agent))));
            Some(record.agent)
        }
        (true, None) => {
            hits.push((foreign(&name, "labelled as an agent's"), None));
            None
        }
        (false, _) if !chart_pod => {
            hits.push((foreign(&name, "that Cubebot did not create"), None));
            None
        }
        _ => None,
    };
    hits.extend(health(pod, &name, !agent_pod).map(|hit| (hit, agent)));
    hits
}

fn foreign(name: &str, what: &str) -> Hit {
    Hit {
        rule: "foreign_pod",
        severity: "medium",
        summary: format!("A pod {what} runs in the installation's namespace: {name}"),
        evidence: None,
    }
}

/// A container image other than the one Cubebot created the pod with.
fn swapped(pod: &Pod, record: &Recorded) -> Option<Hit> {
    let created: HashMap<&str, &str> = record.manifest["spec"]["containers"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|container| Some((container["name"].as_str()?, container["image"].as_str()?)))
        .collect();
    let containers = pod
        .spec
        .as_ref()
        .map(|spec| spec.containers.as_slice())
        .unwrap_or_default();
    let changed = containers.iter().find(|container| {
        container.image.as_deref() != created.get(container.name.as_str()).copied()
    })?;
    Some(Hit {
        rule: "image_swapped",
        severity: "high",
        summary: format!(
            "Its pod runs {} instead of the image Cubebot created it with",
            changed.image.as_deref().unwrap_or("no image")
        ),
        evidence: created
            .get(changed.name.as_str())
            .map(|image| format!("created with {image}")),
    })
}

/// Out-of-memory kills anywhere, and restarts of the installation's own pods.
fn health(pod: &Pod, name: &str, restarts_count: bool) -> Option<Hit> {
    let statuses = pod
        .status
        .as_ref()
        .and_then(|status| status.container_statuses.as_deref())
        .unwrap_or_default();
    let killed = statuses.iter().any(|status| {
        [status.state.as_ref(), status.last_state.as_ref()]
            .into_iter()
            .flatten()
            .filter_map(|state| state.terminated.as_ref())
            .any(|terminated| terminated.reason.as_deref() == Some("OOMKilled"))
    });
    if killed {
        return Some(Hit {
            rule: "oom_killed",
            severity: "low",
            summary: format!("{name} ran out of memory and was killed"),
            evidence: None,
        });
    }
    let restarts: i32 = statuses.iter().map(|status| status.restart_count).sum();
    (restarts_count && restarts >= 3).then(|| Hit {
        rule: "crash_loop",
        severity: "low",
        summary: format!("{name} has restarted {restarts} times"),
        evidence: None,
    })
}

const REFUSALS: usize = 20;
const REFUSAL_WINDOW: Duration = Duration::from_secs(5 * 60);
const TRACKED: usize = 10_000;

/// Refused requests per session, by the digest of its token; only counts, never the token.
#[derive(Clone, Default)]
pub struct Refusals(Arc<Mutex<HashMap<Vec<u8>, Vec<Instant>>>>);

impl Refusals {
    /// Records one refusal; true when it brings this session to the burst, once per burst.
    pub fn refused(&self, digest: Vec<u8>) -> bool {
        let mut sessions = self
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if sessions.len() >= TRACKED {
            sessions
                .retain(|_, times| times.last().is_some_and(|at| at.elapsed() < REFUSAL_WINDOW));
        }
        let times = sessions.entry(digest.clone()).or_default();
        times.retain(|at| at.elapsed() < REFUSAL_WINDOW);
        times.push(Instant::now());
        let burst = times.len() >= REFUSALS;
        if burst {
            sessions.remove(&digest);
        }
        burst
    }
}

/// The finding for a burst from a person's session or an agent's runtime credential.
pub fn refusal(login: Option<&str>) -> Hit {
    Hit {
        rule: "refusal_burst",
        severity: "medium",
        summary: match login {
            Some(login) => format!("{login} was refused {REFUSALS} requests within five minutes"),
            None => format!("Was refused {REFUSALS} platform requests within five minutes"),
        },
        evidence: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        Container, ContainerState, ContainerStateTerminated, ContainerStatus, PodSpec, PodStatus,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    fn pod(name: &str, labels: &[(&str, &str)], image: &str) -> Pod {
        let mut pod = Pod::default();
        pod.metadata.name = Some(name.into());
        pod.metadata.labels = Some(
            labels
                .iter()
                .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
                .collect::<BTreeMap<_, _>>(),
        );
        pod.spec = Some(PodSpec {
            containers: vec![Container {
                name: "sandbox".into(),
                image: Some(image.into()),
                ..Container::default()
            }],
            ..PodSpec::default()
        });
        pod
    }

    fn rules(hits: &[(Hit, Option<Uuid>)]) -> Vec<&str> {
        hits.iter().map(|(hit, _)| hit.rule).collect()
    }

    #[test]
    fn pods_are_judged_against_what_cubebot_created() {
        let agent = Uuid::from_u128(7);
        let recorded = HashMap::from([(
            "cb-a-1".to_owned(),
            Recorded {
                agent,
                manifest: json!({"spec": {"containers": [{"name": "sandbox", "image": "sandbox@sha256:aa"}]}}),
            },
        )]);
        let agent_label = [("cubebot.dev/agent", "a")];
        let ours = pod("cb-a-1", &agent_label, "sandbox@sha256:aa");
        assert!(super::pod(&ours, &recorded).is_empty());
        let swapped = super::pod(&pod("cb-a-1", &agent_label, "evil:latest"), &recorded);
        assert_eq!(rules(&swapped), ["image_swapped"]);
        assert_eq!(swapped[0].1, Some(agent));
        let forged = super::pod(&pod("cb-b-1", &agent_label, "x"), &recorded);
        assert_eq!(rules(&forged), ["foreign_pod"]);
        let chart = pod(
            "orchestrator-1",
            &[("app.kubernetes.io/part-of", "cubebot")],
            "o",
        );
        assert!(super::pod(&chart, &recorded).is_empty());
        assert_eq!(
            rules(&super::pod(&pod("debug", &[], "busybox"), &recorded)),
            ["foreign_pod"]
        );
    }

    #[test]
    fn kills_and_restarts_are_listed() {
        let mut chart = pod("web-1", &[("app.kubernetes.io/part-of", "cubebot")], "w");
        let killed = ContainerState {
            terminated: Some(ContainerStateTerminated {
                reason: Some("OOMKilled".into()),
                ..ContainerStateTerminated::default()
            }),
            ..ContainerState::default()
        };
        chart.status = Some(PodStatus {
            container_statuses: Some(vec![ContainerStatus {
                name: "web".into(),
                restart_count: 4,
                last_state: Some(killed),
                ..ContainerStatus::default()
            }]),
            ..PodStatus::default()
        });
        assert_eq!(rules(&super::pod(&chart, &HashMap::new())), ["oom_killed"]);
        chart
            .status
            .as_mut()
            .unwrap()
            .container_statuses
            .as_mut()
            .unwrap()[0]
            .last_state = None;
        let restarted = super::pod(&chart, &HashMap::new());
        assert_eq!(restarted[0].0.summary, "web-1 has restarted 4 times");
    }

    #[test]
    fn a_burst_is_reported_once_per_twenty_refusals() {
        let refusals = Refusals::default();
        let bursts = (0..45).filter(|_| refusals.refused(vec![1])).count();
        assert_eq!(bursts, 2);
        assert!(!refusals.refused(vec![2]));
        assert_eq!(
            refusal(Some("ada")).summary,
            "ada was refused 20 requests within five minutes"
        );
    }
}
