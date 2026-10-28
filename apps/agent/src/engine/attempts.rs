//! Local crash recovery for tool effects. A call is durable before its command starts and is
//! never run again; after a crash it is reported once, with its outcome or as uncertain.
use crate::journal::Journal;
use cubebot_protocol::{runtime::TurnEvent, terminal::Outcome};
use serde::{Deserialize, Serialize};
use std::{io, path::Path};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Entry {
    pub turn: Uuid,
    pub sequence: u64,
    pub event: TurnEvent,
}
pub(super) struct Store {
    journal: Journal,
}
impl Store {
    pub fn open(path: &Path) -> io::Result<Self> {
        Ok(Self {
            journal: Journal::open(path)?,
        })
    }
    /// Returns the journal sequence that acknowledges this entry.
    pub fn record(&mut self, entry: &Entry) -> io::Result<u64> {
        self.journal.append(&serde_json::to_vec(entry)?)
    }
    /// Called once the ledger has the entry and everything before it.
    pub fn acknowledge(&mut self, through: u64) -> io::Result<()> {
        self.journal.acknowledge(through)
    }
    /// Unacknowledged entries in order. A call without a result gets an uncertain result in the
    /// sequence reserved for it, acknowledged with the call.
    pub fn pending(&self) -> io::Result<Vec<(Entry, u64)>> {
        let mut pending = Vec::new();
        for record in self.journal.pending() {
            let entry: Entry = serde_json::from_slice(&record.payload)?;
            // Calls are serial: a result always follows its own call.
            if !matches!(entry.event, TurnEvent::ToolResult { .. }) {
                settle(&mut pending);
            }
            pending.push((entry, record.sequence));
        }
        settle(&mut pending);
        Ok(pending)
    }
}
fn settle(pending: &mut Vec<(Entry, u64)>) {
    if let Some((
        Entry {
            turn,
            sequence,
            event: TurnEvent::ToolCall { id, .. },
        },
        journal,
    )) = pending.last()
    {
        let uncertain = Entry {
            turn: *turn,
            sequence: sequence + 1,
            event: TurnEvent::ToolResult {
                id: *id,
                outcome: Outcome::Uncertain,
            },
        };
        let journal = *journal;
        pending.push((uncertain, journal));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(turn: Uuid, sequence: u64) -> Entry {
        Entry {
            turn,
            sequence,
            event: TurnEvent::ToolCall {
                id: Uuid::new_v4(),
                name: "terminal".into(),
                input: serde_json::json!({"command":"echo once"}),
            },
        }
    }

    #[test]
    fn restart_reports_unfinished_calls_as_uncertain_and_forgets_acknowledged_ones() {
        let root = std::env::temp_dir().join(format!("cubebot-attempts-{}", Uuid::new_v4()));
        let turn = Uuid::new_v4();
        let mut store = Store::open(&root).unwrap();
        let first = call(turn, 4);
        let journal = store.record(&first).unwrap();
        let TurnEvent::ToolCall { id, .. } = first.event else {
            unreachable!()
        };
        let finished = Entry {
            turn,
            sequence: 5,
            event: TurnEvent::ToolResult {
                id,
                outcome: Outcome::Exited {
                    code: Some(0),
                    output: String::new(),
                },
            },
        };
        let done = store.record(&finished).unwrap();
        store.record(&call(turn, 6)).unwrap();
        drop(store);

        let mut store = Store::open(&root).unwrap();
        let pending = store.pending().unwrap();
        assert_eq!(pending.len(), 4);
        assert_eq!(pending[0], (first, journal));
        assert_eq!(pending[1], (finished, done));
        assert_eq!(pending[3].0.sequence, 7);
        assert!(matches!(
            pending[3].0.event,
            TurnEvent::ToolResult {
                outcome: Outcome::Uncertain,
                ..
            }
        ));
        assert_eq!(pending[3].1, pending[2].1);
        store.acknowledge(done).unwrap();
        assert_eq!(store.pending().unwrap().len(), 2);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
