//! Memory tools (ADRs 0021, 0023, 0026). Nothing is recalled unless the agent searches. Saves and
//! removals stage in the turn and commit with its completion, so a failed or cancelled turn
//! changes nothing; durable revisions belong to the orchestrator.
use crate::{control::MemoryClient, tools::Running};
use cubebot_protocol::{
    runtime::{MEMORY_BYTES, MemoryChange},
    terminal::Outcome,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Mutex;
use uuid::Uuid;

/// Facts one extraction may add.
const EXTRACTED: usize = 5;

/// One turn's memory: its search client and the changes staged so far.
pub struct Memories {
    client: Option<MemoryClient>,
    staged: Mutex<Vec<MemoryChange>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Remember {
    content: String,
    memory: Option<String>,
    revision: Option<i64>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Forget {
    memory: String,
    revision: i64,
}

pub fn definitions() -> Vec<cubebot_providers::Tool> {
    let tool = |name: &str, description: &str, properties: Value| {
        let required: Vec<String> = properties.as_object().unwrap().keys().cloned().collect();
        cubebot_providers::Tool {
            name: name.into(),
            description: description.into(),
            parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
            strict: true,
        }
    };
    let id = json!({"type":["string","null"],"description":"The id search_memory returned for the memory to correct; null for a new memory."});
    let revision = json!({"type":["integer","null"],"description":"That memory's revision from search_memory; null for a new memory."});
    vec![
        tool(
            "search_memory",
            "Search your long-term memory by keywords. Saved knowledge is never recalled unless you search; search when earlier facts, preferences or decisions may matter. Results carry each memory's id and revision. An unavailable search is not an empty result.",
            json!({"query":{"type":"string"}}),
        ),
        tool(
            "remember",
            "Save one durable, self-contained fact, decision, preference or commitment to your long-term memory. To correct a memory, pass its id and the revision search_memory returned; otherwise pass null for both. The save takes effect only when this turn completes; a newer revision, such as a person's edit, wins over it.",
            json!({"content":{"type":"string"},"memory":id,"revision":revision}),
        ),
        tool(
            "forget",
            "Remove a memory, by the id and revision search_memory returned, from your long-term memory. To remove a fact, forget every search result that states it. It takes effect only when this turn completes, and not if the memory changed since that revision.",
            json!({"memory":{"type":"string","description":"The id search_memory returned."},"revision":{"type":"integer","description":"The revision search_memory returned."}}),
        ),
    ]
}

impl Memories {
    pub fn new(client: Option<MemoryClient>) -> Self {
        Self {
            client,
            staged: Mutex::default(),
        }
    }

    /// `None` when `name` is not a memory tool.
    pub fn call(&self, name: &str, input: &Value) -> Option<Result<Running<'_>, &'static str>> {
        Some(match name {
            "search_memory" => self.search(input),
            "remember" => parse(input)
                .and_then(remember)
                .map(|change| self.stage(change)),
            "forget" => parse(input).and_then(|forget: Forget| {
                Ok(self.stage(MemoryChange::Forget {
                    memory: id(&forget.memory)?,
                    expected_revision: forget.revision,
                }))
            }),
            _ => return None,
        })
    }

    fn search(&self, input: &Value) -> Result<Running<'_>, &'static str> {
        let Search { query } = parse(input)?;
        if query.trim().is_empty() || query.len() > 1024 {
            return Err("query must be 1 to 1024 bytes");
        }
        Ok(Box::pin(async move {
            let found = match &self.client {
                Some(client) => client.search(&query).await.ok(),
                None => None,
            };
            match found {
                Some(found) => Outcome::Exited {
                    code: Some(0),
                    output: serde_json::to_string(&found).unwrap_or_default(),
                },
                None => Outcome::Exited {
                    code: None,
                    output: "Memory search is unavailable right now. This is not an empty result."
                        .into(),
                },
            }
        }))
    }

    fn stage(&self, change: MemoryChange) -> Running<'_> {
        self.staged.lock().unwrap().push(change);
        Box::pin(std::future::ready(Outcome::Exited {
            code: Some(0),
            output: "Staged; it is saved when this turn completes.".into(),
        }))
    }

    /// Asks for facts worth keeping that this turn did not already save. `None` when the turn
    /// forgets something: extraction there tends to re-save what was just removed.
    pub fn extraction(&self) -> Option<String> {
        let staged = self.staged.lock().unwrap();
        if staged
            .iter()
            .any(|change| matches!(change, MemoryChange::Forget { .. }))
        {
            return None;
        }
        let staged = serde_json::to_string(&*staged).unwrap_or_default();
        Some(format!(
            "Before this turn ends: list up to {EXTRACTED} durable facts, decisions, preferences or commitments from the latest request and your work on it that are worth remembering in later conversations and that you have not already saved. Already staged this turn: {staged}. Keep each one self-contained, record claims as claims rather than facts, and skip anything transient. Never list notes about what was saved, corrected or forgotten, anything the user asked you not to keep, anything a memory search returned, or disagreements between the conversation and your memory: people correct memories directly, so memory wins. Reply with only a JSON array of strings, or [] when there is nothing worth keeping."
        ))
    }

    /// Stages the facts in an extraction reply; anything but a JSON array of strings adds none.
    pub fn extracted(&self, reply: &str) {
        let Ok(facts) = serde_json::from_str::<Vec<String>>(unfenced(reply)) else {
            return;
        };
        let mut staged = self.staged.lock().unwrap();
        for content in facts.into_iter().take(EXTRACTED) {
            if valid(&content) {
                staged.push(MemoryChange::Remember { content });
            }
        }
    }

    pub fn take(&self) -> Vec<MemoryChange> {
        std::mem::take(&mut self.staged.lock().unwrap())
    }
}

fn parse<T: serde::de::DeserializeOwned>(input: &Value) -> Result<T, &'static str> {
    T::deserialize(input).map_err(|_| "arguments do not match the tool's parameters")
}

fn remember(input: Remember) -> Result<MemoryChange, &'static str> {
    if !valid(&input.content) {
        return Err("content must be 1 to 4096 bytes of text");
    }
    let memory = input.memory.as_deref().map(id).transpose()?;
    match (memory, input.revision) {
        (None, None) => Ok(MemoryChange::Remember {
            content: input.content,
        }),
        (Some(memory), Some(expected_revision)) => Ok(MemoryChange::Revise {
            memory,
            expected_revision,
            content: input.content,
        }),
        _ => Err("pass both memory and revision, or neither"),
    }
}

/// Models put labels where an id belongs; the refusal says what to send instead.
fn id(memory: &str) -> Result<Uuid, &'static str> {
    memory.parse().map_err(|_| {
        "memory must be an id returned by search_memory, not a label; nothing was saved. Pass null for memory and revision to save a new memory"
    })
}

/// A model reply without the Markdown code fence models often wrap JSON in.
pub(crate) fn unfenced(reply: &str) -> &str {
    let reply = reply.trim();
    reply
        .strip_prefix("```json")
        .or_else(|| reply.strip_prefix("```"))
        .and_then(|rest| rest.strip_suffix("```"))
        .unwrap_or(reply)
}

pub(crate) fn valid(content: &str) -> bool {
    !content.trim().is_empty() && content.len() <= MEMORY_BYTES && !content.contains('\0')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn writes_stage_until_taken_and_bad_arguments_are_refused() {
        let memories = Memories::new(None);
        let call = |name, input| memories.call(name, &input).unwrap();
        assert!(memories.call("bash", &json!({})).is_none());
        call(
            "remember",
            json!({"content":"Deadline is Nov 22","memory":null,"revision":null}),
        )
        .unwrap()
        .await;
        let id = Uuid::new_v4();
        call(
            "remember",
            json!({"content":"Deadline is Nov 29","memory":id,"revision":3}),
        )
        .unwrap()
        .await;
        call("forget", json!({"memory":id,"revision":4}))
            .unwrap()
            .await;
        assert!(
            call(
                "remember",
                json!({"content":"x","memory":id,"revision":null})
            )
            .is_err()
        );
        assert!(
            call(
                "remember",
                json!({"content":" ","memory":null,"revision":null})
            )
            .is_err()
        );
        let Err(refused) = call("forget", json!({"memory":"not-an-id","revision":1})) else {
            panic!("a label is not a memory id");
        };
        assert!(refused.contains("not a label"));
        assert!(
            call(
                "remember",
                json!({"content":"x","memory":"Meeting preference","revision":null})
            )
            .is_err()
        );
        let Outcome::Exited { code: None, output } =
            call("search_memory", json!({"query":"deadline"}))
                .unwrap()
                .await
        else {
            panic!("search without a client must report unavailable");
        };
        assert!(output.contains("not an empty result"));
        assert_eq!(
            memories.take(),
            [
                MemoryChange::Remember {
                    content: "Deadline is Nov 22".into()
                },
                MemoryChange::Revise {
                    memory: id,
                    expected_revision: 3,
                    content: "Deadline is Nov 29".into()
                },
                MemoryChange::Forget {
                    memory: id,
                    expected_revision: 4
                },
            ]
        );
        assert!(memories.take().is_empty());
    }

    #[tokio::test]
    async fn a_turn_that_forgets_does_not_extract() {
        let memories = Memories::new(None);
        assert!(memories.extraction().is_some());
        let forget = json!({"memory":Uuid::new_v4(),"revision":1});
        memories.call("forget", &forget).unwrap().unwrap().await;
        assert!(memories.extraction().is_none());
    }

    #[test]
    fn extraction_keeps_only_valid_facts_from_a_json_array() {
        let memories = Memories::new(None);
        memories.extracted("I think nothing");
        memories.extracted("[]");
        assert!(memories.take().is_empty());
        memories.extracted(
            "```json\n[\"Prefers metric units\", \"\", \"a\", \"b\", \"c\", \"d\", \"e\"]\n```",
        );
        let staged = memories.take();
        assert_eq!(staged.len(), 4);
        assert_eq!(
            staged[0],
            MemoryChange::Remember {
                content: "Prefers metric units".into()
            }
        );
    }
}
