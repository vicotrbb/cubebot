//! One dream run (ADR 0025): with every lane idle, the agent asks its model to consolidate its own
//! memories, and to write down a recurring procedure from its recent turns as a new or revised
//! skill (ADR 0055). It commits the proposals one at a time, each naming the revision it read. A
//! channel message stops the run before its next commit; what was committed stays.
use crate::control::MemoryClient;
use cubebot_protocol::runtime::{
    Dream, DreamEnd, DreamOutcome, MemoryChange, ProviderKind, REPORT_BYTES, SkillChange,
};
use cubebot_providers::{Anthropic, Error, Event, Message, OpenAi, Provider, Request};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::watch;
use uuid::Uuid;

/// Changes one run may commit.
const CHANGES: usize = 50;
/// Skills one run may publish.
const SKILLS: usize = 3;

const SYSTEM: &str = "You maintain your own long-term memory while you are idle.";

/// The model's reply; each change is parsed on its own so one malformed change drops alone.
#[derive(Deserialize)]
struct Reply {
    changes: Vec<serde_json::Value>,
    #[serde(default)]
    skills: Vec<serde_json::Value>,
    report: String,
}

struct Plan {
    changes: Vec<MemoryChange>,
    skills: Vec<SkillChange>,
    report: String,
}

#[tracing::instrument(skip_all, fields(run = %run))]
pub(super) async fn run(client: MemoryClient, run: Uuid, stop: watch::Receiver<bool>) {
    let dream = match client.start_dream(run).await {
        Ok(Some(dream)) => dream,
        Ok(None) => return,
        Err(error) => return eprintln!("Dreaming unavailable: {error}"),
    };
    let (outcome, mut report) = dream_on(&client, run, &dream, stop).await;
    while report.len() > REPORT_BYTES {
        report.pop();
    }
    if let Err(error) = client
        .finish_dream(run, &DreamEnd { outcome, report })
        .await
    {
        eprintln!("Dream outcome unreported: {error}");
    }
}

async fn dream_on(
    client: &MemoryClient,
    run: Uuid,
    dream: &Dream,
    mut stop: watch::Receiver<bool>,
) -> (DreamOutcome, String) {
    let plan = tokio::select! {
        plan = plan(dream) => plan,
        _ = stop.wait_for(|stopped| *stopped) => return (DreamOutcome::Yielded, String::new()),
    };
    let plan = match plan {
        Ok(plan) => plan,
        Err(reason) => return (DreamOutcome::Failed, reason),
    };
    for change in plan.changes.iter().take(CHANGES) {
        if *stop.borrow() {
            return (DreamOutcome::Yielded, plan.report);
        }
        // A closed run or an unreachable ledger ends the run; a later run retries from there.
        if !matches!(client.commit_dream(run, change).await, Ok(true)) {
            return (DreamOutcome::Failed, plan.report);
        }
    }
    for skill in plan.skills.iter().take(SKILLS) {
        if *stop.borrow() {
            return (DreamOutcome::Yielded, plan.report);
        }
        if !matches!(client.commit_dream_skill(run, skill).await, Ok(true)) {
            return (DreamOutcome::Failed, plan.report);
        }
    }
    (DreamOutcome::Completed, plan.report)
}

/// The model's proposed changes, or why there are none; the reason becomes the run's report.
#[tracing::instrument(skip_all, fields(provider = ?dream.provider, model = %dream.model))]
async fn plan(dream: &Dream) -> Result<Plan, String> {
    let messages = [Message::User(prompt(dream))];
    let request = Request {
        model: &dream.model,
        key: &dream.api_key,
        system: SYSTEM,
        messages: &messages,
        tools: &[],
        tools_disabled: true,
        max_output_tokens: None,
    };
    let reply = match dream.provider {
        ProviderKind::Openai => {
            let provider = super::turn::configured(OpenAi::new, OpenAi::for_loopback_tests);
            reply(provider, &request).await
        }
        ProviderKind::Anthropic => {
            let provider = super::turn::configured(Anthropic::new, Anthropic::for_loopback_tests);
            reply(provider, &request).await
        }
    };
    let reply = reply.map_err(|error| format!("The model call failed: {error:?}."))?;
    parse(&reply).ok_or_else(|| "The model's reply was not a JSON plan.".into())
}

async fn reply<P: Provider>(
    provider: Result<P, Error>,
    request: &Request<'_>,
) -> Result<String, Error> {
    let provider = provider?;
    let mut events = std::pin::pin!(provider.stream(request));
    while let Some(event) = events.next().await {
        if let Event::Done { reply, .. } = event? {
            return Ok(reply.text);
        }
    }
    Err(Error::Unavailable)
}

fn prompt(dream: &Dream) -> String {
    let memories = serde_json::to_string(&dream.memories).unwrap_or_default();
    let turns = serde_json::to_string(&dream.turns).unwrap_or_default();
    let skills = serde_json::to_string(&dream.skills).unwrap_or_default();
    format!(
        "Your memories, each with its id and revision: {memories}\n\nYour latest conversation turns, oldest first, cut short: {turns}\n\nYour installed skills, each with its revision and files: {skills}\n\nFirst, improve your memories. Merge duplicates: revise the one to keep so it states everything the others did, then forget the others. Reconcile contradictions when one claim is clearly newer or better supported; otherwise leave them and say so in the report. Rewrite stale or vague summaries, and make each memory self-contained, using the words someone would search for. Never invent or drop a detail, and leave good memories alone. Then, only when the turns show a multi-step procedure that worked and will likely recur, write it as a skill, or improve an installed skill the turns showed lacking. A skill is a package of files: SKILL.md starts with front matter \"---\\nname: lowercase-hyphenated-name\\ndescription: what it does and when to use it\\n---\\n\" followed by the steps and how to verify them; optional files go under scripts/ or references/, and the steps refer to them by those relative paths. Generalize the procedure: never include credentials, secrets or personal details from the turns. Most runs write no skill. Reply with only a JSON object {{\"changes\":[...],\"skills\":[...],\"report\":\"...\"}}. Each change is {{\"op\":\"revise\",\"memory\":id,\"expected_revision\":revision,\"content\":text}} or {{\"op\":\"forget\",\"memory\":id,\"expected_revision\":revision}}. Each skill is {{\"expected_revision\":null,\"files\":{{\"SKILL.md\":text}}}} for a new one, or the whole revised package with expected_revision set to its installed revision. The report briefly says what you changed, which skill you wrote and why, and what you left unresolved. With nothing to improve, reply {{\"changes\":[],\"skills\":[],\"report\":\"No changes needed.\"}}."
    )
}

/// A plan with only valid changes; anything but the requested JSON object is no plan.
fn parse(reply: &str) -> Option<Plan> {
    let reply: Reply = serde_json::from_str(crate::memory::unfenced(reply)).ok()?;
    let changes = reply
        .changes
        .into_iter()
        .filter_map(|change| serde_json::from_value(change).ok())
        .filter(|change| match change {
            MemoryChange::Remember { content } | MemoryChange::Revise { content, .. } => {
                crate::memory::valid(content)
            }
            MemoryChange::Forget { .. } => true,
        })
        .collect();
    let skills = reply
        .skills
        .into_iter()
        .filter_map(|skill| serde_json::from_value(skill).ok())
        .collect();
    Some(Plan {
        changes,
        skills,
        report: reply.report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plan_keeps_valid_changes_and_refuses_anything_else() {
        let id = Uuid::new_v4();
        let reply = format!(
            "```json\n{{\"changes\":[{{\"op\":\"revise\",\"memory\":\"{id}\",\"expected_revision\":2,\"content\":\"Deadline is Nov 29\"}},{{\"op\":\"revise\",\"memory\":\"{id}\",\"expected_revision\":2,\"content\":\" \"}},{{\"op\":\"forget\",\"memory\":\"{id}\",\"expected_revision\":1}},{{\"op\":\"forget\",\"memory\":\"{id}\",\"expected_revision\":\"1\"}}],\"report\":\"Merged two deadlines.\"}}\n```"
        );
        let plan = parse(&reply).unwrap();
        assert_eq!(plan.report, "Merged two deadlines.");
        assert_eq!(
            plan.changes,
            [
                MemoryChange::Revise {
                    memory: id,
                    expected_revision: 2,
                    content: "Deadline is Nov 29".into()
                },
                MemoryChange::Forget {
                    memory: id,
                    expected_revision: 1
                },
            ]
        );
        assert!(plan.skills.is_empty());
        let reply = r#"{"changes":[],"skills":[{"expected_revision":null,"files":{"SKILL.md":"---\nname: disk-triage\ndescription: x\n---\n"}},{"files":"x"}],"report":"Wrote a skill."}"#;
        let plan = parse(reply).unwrap();
        assert_eq!(plan.skills.len(), 1);
        assert_eq!(plan.skills[0].expected_revision, None);
        assert!(parse("I merged them.").is_none());
        assert!(parse("[]").is_none());
    }
}
