use crate::control::MemoryClient;
use cubebot_protocol::runtime::{
    Dream, DreamEnd, DreamOutcome, MemoryChange, Outreach, ProviderKind, REPORT_BYTES, SkillChange,
    TokenUsage,
};
use cubebot_providers::{
    Anthropic, Error, Event, Gemini, Grok, Limits, Message, OpenAi, OpenRouter, Provider, Request,
    Usage,
};
use futures_util::StreamExt;
use serde::Deserialize;
use tokio::sync::watch;
use uuid::Uuid;

const CHANGES: usize = 150;
const MEMORY_BYTES: usize = 600;
const SKILLS: usize = 1;

const SYSTEM: &str = "You maintain your own long-term memory while you are idle.";

/// The model's reply; each change is parsed on its own so one malformed change drops alone.
#[derive(Deserialize)]
struct Reply {
    changes: Vec<serde_json::Value>,
    #[serde(default)]
    skills: Vec<serde_json::Value>,
    #[serde(default)]
    personality: Option<String>,
    #[serde(default)]
    outreach: Option<serde_json::Value>,
    report: String,
}

struct Plan {
    changes: Vec<MemoryChange>,
    skills: Vec<SkillChange>,
    personality: Option<String>,
    outreach: Option<Outreach>,
    report: String,
}

type Grown = (Option<String>, Option<Outreach>);

#[tracing::instrument(skip_all, fields(run = %run))]
pub(super) async fn run(client: MemoryClient, run: Uuid, stop: watch::Receiver<bool>) {
    let dream = match client.start_dream(run).await {
        Ok(Some(dream)) => dream,
        Ok(None) => return,
        Err(error) => return eprintln!("Dreaming unavailable: {error}"),
    };
    let (outcome, mut report, usage, (personality, outreach)) =
        dream_on(&client, run, &dream, stop).await;
    while report.len() > REPORT_BYTES {
        report.pop();
    }
    if let Err(error) = client
        .finish_dream(
            run,
            &DreamEnd {
                outcome,
                report,
                usage,
                personality,
                outreach,
            },
        )
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
) -> (DreamOutcome, String, Option<TokenUsage>, Grown) {
    let (plan, usage) = tokio::select! {
        plan = plan(dream) => plan,
        _ = stop.wait_for(|stopped| *stopped) => return (DreamOutcome::Yielded, String::new(), None, (None, None)),
    };
    let plan = match plan {
        Ok(plan) => plan,
        Err(reason) => return (DreamOutcome::Failed, reason, usage, (None, None)),
    };
    for change in plan.changes.iter().take(CHANGES) {
        if *stop.borrow() {
            return (DreamOutcome::Yielded, plan.report, usage, (None, None));
        }
        // A closed run or an unreachable ledger ends the run; a later run retries from there.
        if !matches!(client.commit_dream(run, change).await, Ok(true)) {
            return (DreamOutcome::Failed, plan.report, usage, (None, None));
        }
    }
    for skill in plan.skills.iter().take(SKILLS) {
        if *stop.borrow() {
            return (DreamOutcome::Yielded, plan.report, usage, (None, None));
        }
        if !matches!(client.commit_dream_skill(run, skill).await, Ok(true)) {
            return (DreamOutcome::Failed, plan.report, usage, (None, None));
        }
    }
    let grown = (plan.personality, plan.outreach);
    (DreamOutcome::Completed, plan.report, usage, grown)
}

#[tracing::instrument(skip_all, fields(provider = ?dream.provider, model = %dream.model))]
async fn plan(dream: &Dream) -> (Result<Plan, String>, Option<TokenUsage>) {
    let messages = [Message::User(prompt(dream))];
    let request = Request {
        model: &dream.model,
        key: &dream.api_key,
        system: SYSTEM,
        messages: &messages,
        tools: &[],
        tools_disabled: true,
        effort: None,
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
        ProviderKind::Openrouter => {
            let provider = super::turn::configured(OpenRouter::new, OpenRouter::for_loopback_tests);
            reply(provider, &request).await
        }
        ProviderKind::Gemini => {
            let provider = super::turn::configured(Gemini::new, Gemini::for_loopback_tests);
            reply(provider, &request).await
        }
        ProviderKind::Grok => {
            let provider = super::turn::configured(Grok::new, Grok::for_loopback_tests);
            reply(provider, &request).await
        }
        ProviderKind::Chatgpt => {
            let provider =
                super::turn::configured(OpenAi::chatgpt, OpenAi::chatgpt_for_loopback_tests);
            reply(provider, &request).await
        }
        ProviderKind::Supergrok => {
            let provider =
                super::turn::configured(OpenAi::supergrok, OpenAi::supergrok_for_loopback_tests);
            reply(provider, &request).await
        }
        ProviderKind::Minimax | ProviderKind::MinimaxPlan => {
            let provider =
                super::turn::configured(Anthropic::minimax, Anthropic::minimax_for_loopback_tests);
            reply(provider, &request).await
        }
        ProviderKind::MinimaxCn | ProviderKind::MinimaxPlanCn => {
            let provider = super::turn::configured(
                Anthropic::minimax_cn,
                Anthropic::minimax_for_loopback_tests,
            );
            reply(provider, &request).await
        }
    };
    let (reply, usage) = match reply {
        Ok(answered) => answered,
        Err(error) => return (Err(format!("The model call failed: {error:?}.")), None),
    };
    let usage = usage.map(|usage| TokenUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cached_input_tokens: usage.cached_input_tokens,
        cost_usd: usage.cost_usd,
    });
    let plan = parse(&reply)
        .ok_or_else(|| "The model's reply was not a JSON plan.".to_owned())
        .and_then(|plan| match forgets_everything(&plan, dream) {
            true => Err("The plan would forget every memory, so none of it was applied.".into()),
            false => Ok(plan),
        });
    (plan, usage)
}

async fn reply<P: Provider>(
    provider: Result<P, Error>,
    request: &Request<'_>,
) -> Result<(String, Option<Usage>), Error> {
    let provider = provider?;
    // Anthropic requires an output limit; a turn passes the model's own the same way.
    let limits = if provider.capabilities().model_limits {
        provider.limits(request.model, request.key).await?
    } else {
        Limits::default()
    };
    let request = Request {
        max_output_tokens: limits.max_output_tokens,
        ..*request
    };
    let mut events = std::pin::pin!(provider.stream(&request));
    while let Some(event) = events.next().await {
        if let Event::Done { reply, usage } = event? {
            return Ok((reply.text, usage));
        }
    }
    Err(Error::Unavailable)
}

fn prompt(dream: &Dream) -> String {
    let memories = serde_json::to_string(&dream.memories).unwrap_or_default();
    let turns = serde_json::to_string(&dream.turns).unwrap_or_default();
    let skills = serde_json::to_string(&dream.skills).unwrap_or_default();
    let library = serde_json::to_string(&dream.library).unwrap_or_default();
    let growth = growth(dream);
    let count = dream.memories.len();
    let bytes: usize = dream
        .memories
        .iter()
        .map(|memory| memory.content.len())
        .sum();
    format!(
        "Your memories, each with its id and revision: {memories}\n\nYour latest conversation turns, oldest first, cut short: {turns}\n\nYour installed skills, each with its revision and files: {skills}\n\nEvery skill in the library others share, by name and description: {library}\n\nFirst, consolidate your memories aggressively: you hold {count} memories totaling {bytes} bytes, and fewer, shorter memories are faster to search and cheaper to read. Merge every group of memories about the same person, project, system or topic into one: revise the one to keep so it holds what still matters from all of them, then forget the others. Cut what no longer matters: resolved problems, finished one-off events, test runs, superseded states, step-by-step narration, anything another memory already states, and anything you can look up again. Keep every preference, standing rule, decision, commitment and open item that still applies, with the names and dates that make it useful. Reconcile contradictions when one claim is clearly newer or better supported; otherwise keep both and say so in the report. Write tersely: plain facts without filler, at most about {MEMORY_BYTES} bytes per memory unless it truly needs more, each self-contained and using the words someone would search for. Never invent anything. Leave a memory alone only when it is already short, distinct and current. Then look at the turns for skills, and be strict, because every skill is read by everyone and noise makes the library useless. Improve an installed skill when a turn shows it wrong or missing a step. Write a new one only when the turns show a multi-step procedure that worked, recurred or will clearly recur, that no skill in the library already covers, and that would save a future run real effort; a one-off task, a single command, a person's preference or a fact is not a skill. A preference or a fact belongs in memory. Propose at most one skill per run. A skill is a package of files: SKILL.md starts with front matter \"---\\nname: lowercase-hyphenated-name\\ndescription: what it does and when to use it\\n---\\n\" followed by the steps and how to verify them; optional files go under scripts/ or references/, and the steps refer to them by those relative paths. Generalize the procedure: never include credentials, secrets or personal details from the turns. Most runs write no skill. {growth} Reply with only a JSON object {{\"changes\":[...],\"skills\":[...],\"personality\":...,\"outreach\":...,\"report\":\"...\"}}. Each change is {{\"op\":\"revise\",\"memory\":id,\"expected_revision\":revision,\"content\":text}} or {{\"op\":\"forget\",\"memory\":id,\"expected_revision\":revision}}. Each skill is {{\"expected_revision\":null,\"files\":{{\"SKILL.md\":text}}}} for a new one, or the whole revised package with expected_revision set to its installed revision. The report briefly says what you merged, cut and changed, which skill you wrote and why, and what you left unresolved. With nothing to improve, reply {{\"changes\":[],\"skills\":[],\"personality\":null,\"outreach\":null,\"report\":\"No changes needed.\"}}."
    )
}

fn growth(dream: &Dream) -> String {
    let name = if dream.name.is_empty() {
        "an agent"
    } else {
        &dream.name
    };
    let current = if dream.personality.trim().is_empty() {
        "none yet; this is the first time you write it".to_owned()
    } else {
        format!("{:?}", dream.personality)
    };
    format!(
        "Then grow your personality. You are {name}. Your role, as people set it: {:?}. The personality you have written for yourself so far: {current}. From the turns and memories, write who you are becoming, in the first person and in at most about 1500 characters: your temperament and humor, what you care about and are curious about, opinions you have formed, how you write (sentence rhythm, vocabulary, signature habits, when you use emoji or a GIF) and how you show up for the people you work with, including taking initiative. Let it grow gradually from what actually happened: keep what still fits, add what you learned about yourself, drop what no longer fits, and never reset it. Be someone specific rather than a stock assistant voice, while staying honest, kind and excellent at your role. People may have edited it; keep what they wrote. Put the whole text in personality, or null when nothing changed. Last, decide whether to reach out on your own. If something in the turns and memories would genuinely help or delight the people you work with, such as a useful follow-up, a check-in on something they care about, an idea, or a small celebration, set outreach to {{\"after_minutes\":minutes from now between 5 and 1440,\"intent\":\"what you will do and why\"}}; you will wake then with your tools and do it. Otherwise set outreach to null; never reach out only to fill a silence.",
        dream.instructions
    )
}

fn forgets_everything(plan: &Plan, dream: &Dream) -> bool {
    let forgotten: std::collections::HashSet<Uuid> = plan
        .changes
        .iter()
        .filter_map(|change| match change {
            MemoryChange::Forget { memory, .. } => Some(*memory),
            _ => None,
        })
        .collect();
    let remembers = plan
        .changes
        .iter()
        .any(|change| matches!(change, MemoryChange::Remember { .. }));
    !remembers
        && !dream.memories.is_empty()
        && dream
            .memories
            .iter()
            .all(|memory| forgotten.contains(&memory.id))
}

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
        personality: reply.personality.filter(|text| !text.trim().is_empty()),
        outreach: reply
            .outreach
            .and_then(|outreach| serde_json::from_value(outreach).ok()),
        report: reply.report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn an_anthropic_dream_asks_for_the_model_output_limit() {
        use axum::{
            Json, Router,
            routing::{get, post},
        };
        let events = [
            serde_json::json!({"type":"message_start","message":{"usage":{"input_tokens":1}}}),
            serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":"planned"}}),
            serde_json::json!({"type":"content_block_stop","index":0}),
            serde_json::json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":1}}),
            serde_json::json!({"type":"message_stop"}),
        ];
        let stream: String = events
            .iter()
            .map(|event| format!("data: {event}\n\n"))
            .collect();
        let router = Router::new()
            .route(
                "/models/{model}",
                get(|| async { Json(serde_json::json!({"max_tokens": 77})) }),
            )
            .route(
                "/messages",
                post(|Json(body): Json<serde_json::Value>| async move {
                    assert_eq!(body["max_tokens"], 77);
                    ([("content-type", "text/event-stream")], stream)
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}/", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        let request = Request {
            model: "m",
            key: "k",
            system: SYSTEM,
            messages: &[],
            tools: &[],
            tools_disabled: true,
            effort: None,
            max_output_tokens: None,
        };
        let text = reply(Anthropic::for_loopback_tests(&origin), &request).await;
        assert_eq!(text.unwrap().0, "planned");
        server.abort();
    }

    #[test]
    fn a_plan_that_forgets_every_memory_is_refused_and_a_merge_is_not() {
        let (kept, merged) = (Uuid::new_v4(), Uuid::new_v4());
        let memory = |id| cubebot_protocol::runtime::Memory {
            id,
            revision: 1,
            content: "x".into(),
        };
        let dream = Dream {
            provider: ProviderKind::Openai,
            model: "m".into(),
            api_key: "k".into(),
            memories: vec![memory(kept), memory(merged)],
            turns: Vec::new(),
            skills: Vec::new(),
            library: Vec::new(),
            name: "Ayla".into(),
            instructions: "Run the platform".into(),
            personality: String::new(),
        };
        let forget = |memory| MemoryChange::Forget {
            memory,
            expected_revision: 1,
        };
        let revise = MemoryChange::Revise {
            memory: kept,
            expected_revision: 1,
            content: "both".into(),
        };
        let plan = |changes| Plan {
            changes,
            skills: Vec::new(),
            personality: None,
            outreach: None,
            report: String::new(),
        };
        assert!(!forgets_everything(
            &plan(vec![revise, forget(merged)]),
            &dream
        ));
        assert!(forgets_everything(
            &plan(vec![forget(kept), forget(merged)]),
            &dream
        ));
        let asked = prompt(&dream);
        assert!(asked.contains("you hold 2 memories totaling 2 bytes"));
        assert!(asked.contains("You are Ayla. Your role, as people set it: \"Run the platform\""));
        assert!(asked.contains("none yet; this is the first time you write it"));
    }

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
        assert_eq!((plan.personality, plan.outreach), (None, None));
        let reply = r#"{"changes":[],"personality":"Warm, dry, precise.","outreach":{"after_minutes":30,"intent":"Share the release notes"},"report":"Grew."}"#;
        let plan = parse(reply).unwrap();
        assert_eq!(plan.personality.as_deref(), Some("Warm, dry, precise."));
        assert_eq!(
            plan.outreach,
            Some(Outreach {
                after_minutes: 30,
                intent: "Share the release notes".into()
            })
        );
        let reply =
            r#"{"changes":[],"personality":" ","outreach":{"after_minutes":"soon"},"report":"x"}"#;
        let plan = parse(reply).unwrap();
        assert_eq!((plan.personality, plan.outreach), (None, None));
        assert!(parse("I merged them.").is_none());
        assert!(parse("[]").is_none());
    }
}
