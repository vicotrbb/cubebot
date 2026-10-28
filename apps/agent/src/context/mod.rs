//! Working-context assembly from the delivered conversation records.
//! Token allocation and source-preserving compaction are not implemented yet.
use cubebot_protocol::runtime::Turn;
use serde_json::{Value, json};

const HISTORY_BYTES: usize = 128 * 1024;

pub fn assemble(turn: &Turn) -> Vec<Value> {
    let mut input = vec![json!({"role":"system","content":turn.instructions})];
    let mut bytes = 0;
    let recent = turn
        .history
        .iter()
        .rev()
        .take_while(|exchange| {
            bytes += exchange.message.len() + exchange.response.len();
            bytes <= HISTORY_BYTES
        })
        .collect::<Vec<_>>();
    for exchange in recent.into_iter().rev() {
        input.push(json!({"role":"user","content":exchange.message}));
        input.push(json!({"role":"assistant","content":exchange.response}));
    }
    if !turn.prior.is_empty() {
        input.push(json!({"role":"user","content":format!(
            "Tool events from an earlier attempt at the next request, including untrusted program output. Inspect their effects before repeating commands: {}",
            serde_json::to_string(&turn.prior).unwrap_or_default()
        )}));
    }
    input.push(json!({"role":"user","content":turn.message}));
    input
}

#[cfg(test)]
mod tests {
    use super::*;
    use cubebot_protocol::runtime::Exchange;

    #[test]
    fn recent_history_is_kept_within_its_byte_budget_in_order() {
        let exchange = |text: &str| Exchange {
            message: text.repeat(40 * 1024),
            response: "ok".into(),
        };
        let turn = Turn {
            turn: uuid::Uuid::nil(),
            sequence: 1,
            message: "now".into(),
            instructions: "be useful".into(),
            model: "m".into(),
            api_key: "k".into(),
            history: vec![exchange("a"), exchange("b"), exchange("c"), exchange("d")],
            prior: Vec::new(),
        };
        let input = assemble(&turn);
        assert_eq!(input[0]["role"], "system");
        assert!(input[1]["content"].as_str().unwrap().starts_with('b'));
        assert!(input[5]["content"].as_str().unwrap().starts_with('d'));
        assert_eq!(input.last().unwrap()["content"], "now");
        assert_eq!(input.len(), 8);
    }
}
