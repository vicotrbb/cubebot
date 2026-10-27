use crate::journal::Journal;
use cubebot_protocol::terminal::{Invocation, Outcome};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, io, path::Path};
use uuid::Uuid;

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", deny_unknown_fields)]
enum Record {
    Intent { request: Invocation },
    Outcome { id: Uuid, outcome: Outcome },
}
pub(super) struct Store {
    journal: Journal,
    invocations: HashMap<Uuid, (Invocation, Outcome)>,
}
impl Store {
    pub fn open(path: &Path) -> io::Result<Self> {
        let journal = Journal::open(path)?;
        let mut invocations = HashMap::new();
        for record in journal.pending() {
            match serde_json::from_slice::<Record>(&record.payload)? {
                Record::Intent { request } => {
                    if invocations
                        .insert(request.id, (request, Outcome::Uncertain))
                        .is_some()
                    {
                        return Err(io::Error::other("duplicate terminal intent"));
                    }
                }
                Record::Outcome { id, outcome } => {
                    let entry = invocations
                        .get_mut(&id)
                        .ok_or_else(|| io::Error::other("outcome without intent"))?;
                    entry.1 = outcome;
                }
            }
        }
        Ok(Self {
            journal,
            invocations,
        })
    }
    pub fn lookup(&self, id: Uuid, authority: Uuid) -> io::Result<Outcome> {
        match self.invocations.get(&id) {
            Some((request, outcome)) if request.authority == authority => Ok(outcome.clone()),
            None => Ok(Outcome::Uncertain),
            _ => Err(io::Error::other("terminal authority conflict")),
        }
    }
    pub fn begin(&mut self, request: &Invocation) -> io::Result<Option<Outcome>> {
        if let Some((prior, outcome)) = self.invocations.get(&request.id) {
            if prior != request {
                return Err(io::Error::other("terminal identity conflict"));
            }
            return Ok(Some(outcome.clone()));
        }
        self.append(&Record::Intent {
            request: request.clone(),
        })?;
        self.invocations
            .insert(request.id, (request.clone(), Outcome::Uncertain));
        Ok(None)
    }
    pub fn finish(&mut self, id: Uuid, outcome: &Outcome) -> io::Result<()> {
        self.append(&Record::Outcome {
            id,
            outcome: outcome.clone(),
        })?;
        self.invocations
            .get_mut(&id)
            .ok_or_else(|| io::Error::other("missing terminal intent"))?
            .1 = outcome.clone();
        Ok(())
    }
    fn append(&mut self, record: &Record) -> io::Result<()> {
        self.journal.append(&serde_json::to_vec(record)?)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restart_preserves_uncertainty_and_rejects_changed_identity() {
        let root = std::env::temp_dir().join(format!("cubebot-terminal-{}", Uuid::new_v4()));
        let request = Invocation {
            id: Uuid::new_v4(),
            authority: Uuid::new_v4(),
            command: "echo once".into(),
            seconds: 1,
        };
        let mut store = Store::open(&root).unwrap();
        assert!(store.begin(&request).unwrap().is_none());
        drop(store);
        let mut store = Store::open(&root).unwrap();
        assert_eq!(store.begin(&request).unwrap(), Some(Outcome::Uncertain));
        let mut changed = request.clone();
        changed.command = "echo twice".into();
        assert!(store.begin(&changed).is_err());
        let outcome = Outcome::Exited {
            code: Some(7),
            output: "failure".into(),
        };
        store.finish(request.id, &outcome).unwrap();
        drop(store);
        let mut store = Store::open(&root).unwrap();
        assert_eq!(store.begin(&request).unwrap(), Some(outcome));
        std::fs::remove_dir_all(&root).unwrap();
    }
}
