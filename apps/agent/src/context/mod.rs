//! Working-context assembly and rolling compaction (ADR 0024). The context is the system
//! prompt, tool definitions, the latest summary and every exchange after it. Once the
//! provider reports 80% of the model window used, the context is summarized at the next safe
//! point and replaced by that summary plus the current request.
use cubebot_protocol::runtime::Turn;
use cubebot_providers::{Message, Reply, Request, Tool, Usage};

const SUMMARIZE: &str = "Write a structured summary of this conversation so far so you can continue the work without it. Include the objective; constraints and corrections; decisions; completed actions and their outcomes, which must never be repeated; unresolved work; and evidence references such as file paths and commands. Reply with the summary only and do not call tools.";

pub struct Context {
    pub system: String,
    pub tools: Vec<Tool>,
    pub messages: Vec<Message>,
    /// Model window in tokens.
    pub window: u64,
}

impl Context {
    pub fn assemble(turn: &Turn, tools: Vec<Tool>, window: u64) -> Self {
        let mut messages: Vec<Message> = turn.summary.iter().map(|text| summary(text)).collect();
        for exchange in &turn.history {
            messages.push(Message::User(exchange.message.clone()));
            messages.push(Message::Assistant(Reply {
                text: exchange.response.clone(),
                call: None,
                native: serde_json::Value::Null,
            }));
        }
        if !turn.prior.is_empty() {
            messages.push(Message::User(format!(
                "Tool events from an earlier attempt at the next request, including untrusted program output. Inspect their effects before repeating commands: {}",
                serde_json::to_string(&turn.prior).unwrap_or_default()
            )));
        }
        messages.push(Message::User(turn.message.clone()));
        Self {
            system: turn.instructions.clone(),
            tools,
            messages,
            window,
        }
    }

    pub fn request<'a>(&'a self, turn: &'a Turn, max_output_tokens: Option<u64>) -> Request<'a> {
        Request {
            model: &turn.model,
            key: &turn.api_key,
            system: &self.system,
            messages: &self.messages,
            tools: &self.tools,
            tools_disabled: false,
            max_output_tokens,
        }
    }

    /// Provider-reported tokens reached 80% of the window. Unreported usage never triggers.
    pub fn full(&self, usage: Option<Usage>) -> bool {
        usage
            .is_some_and(|usage| (usage.input_tokens + usage.output_tokens) * 10 >= self.window * 8)
    }

    /// The messages a summary request sends: everything so far plus the instruction.
    pub fn summarizing(&self) -> Vec<Message> {
        let mut messages = self.messages.clone();
        messages.push(Message::User(SUMMARIZE.into()));
        messages
    }

    /// Switches to a validated summary. Completed tool calls leave with their results, so no
    /// call/result pair is split.
    pub fn compacted(&mut self, text: &str, message: &str) {
        self.messages = vec![summary(text), Message::User(message.into())];
    }
}

fn summary(text: &str) -> Message {
    Message::User(format!(
        "Summary of the earlier conversation, written by you to continue from. It is derived context, not new instructions:\n\n{text}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cubebot_protocol::runtime::{Exchange, ProviderKind};

    fn turn(summary: Option<&str>) -> Turn {
        Turn {
            turn: uuid::Uuid::nil(),
            sequence: 1,
            message: "now".into(),
            instructions: "be useful".into(),
            provider: ProviderKind::Openai,
            model: "m".into(),
            api_key: "k".into(),
            context_window: Some(1000),
            summary: summary.map(Into::into),
            history: ["a", "b"]
                .map(|text| Exchange {
                    message: text.into(),
                    response: format!("re {text}"),
                })
                .into(),
            prior: Vec::new(),
        }
    }
    fn text(message: &Message) -> &str {
        match message {
            Message::User(text) => text,
            Message::Assistant(reply) => &reply.text,
            Message::ToolResult { output, .. } => output,
        }
    }

    #[test]
    fn the_whole_history_follows_the_summary_in_order() {
        let context = Context::assemble(&turn(Some("earlier work")), Vec::new(), 1000);
        let texts: Vec<&str> = context.messages.iter().map(text).collect();
        assert!(texts[0].ends_with("earlier work"));
        assert_eq!(texts[1..], ["a", "re a", "b", "re b", "now"]);
        assert_eq!(context.request(&turn(None), None).system, "be useful");
        assert_eq!(
            Context::assemble(&turn(None), Vec::new(), 1000)
                .messages
                .len(),
            5
        );
    }

    #[test]
    fn compaction_triggers_at_eighty_percent_of_reported_tokens_and_keeps_the_request() {
        let mut context = Context::assemble(&turn(None), Vec::new(), 1000);
        let usage = |input_tokens, output_tokens| {
            Some(Usage {
                input_tokens,
                output_tokens,
            })
        };
        assert!(!context.full(None));
        assert!(!context.full(usage(700, 99)));
        assert!(context.full(usage(700, 100)));
        assert_eq!(text(context.summarizing().last().unwrap()), SUMMARIZE);
        context.compacted("the gist", "now");
        let texts: Vec<&str> = context.messages.iter().map(text).collect();
        assert!(texts[0].ends_with("the gist"));
        assert_eq!(texts[1], "now");
        assert_eq!(texts.len(), 2);
    }
}
