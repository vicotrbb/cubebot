use super::attendants::Note;
use super::wire::Server;
use crate::agents::AgentId;
use crate::speech::{self, Heard, Language, Listener, Provider, Speech};
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

const BACKLOG: usize = 128;

pub(super) struct Preparing {
    updates: mpsc::UnboundedSender<Note>,
    reported: AtomicBool,
}

impl Preparing {
    pub(super) fn new(updates: mpsc::UnboundedSender<Note>) -> Self {
        Self {
            updates,
            reported: AtomicBool::new(false),
        }
    }

    pub(super) fn report(&self, model: &'static str, received: u64, total: u64) {
        self.reported.store(true, Ordering::SeqCst);
        let _ = self.updates.send(Note::Announce(Server::Preparing {
            model: model.to_owned(),
            received,
            total,
        }));
    }

    pub(super) fn done(&self) {
        if self.reported.load(Ordering::SeqCst) {
            self.report("", 0, 0);
        }
    }
}

pub(super) struct Ears {
    pub speech: Speech,
    pub agents: Vec<AgentId>,
    pub provider: Provider,
    pub language: Language,
    pub notes: mpsc::UnboundedSender<Note>,
}

impl Ears {
    pub(super) fn open(self, slot: u16) -> mpsc::Sender<Vec<f32>> {
        let (audio, frames) = mpsc::channel(BACKLOG);
        tokio::spawn(self.listen(slot, frames));
        audio
    }

    #[tracing::instrument(skip_all, fields(slot))]
    async fn listen(self, slot: u16, frames: mpsc::Receiver<Vec<f32>>) {
        let preparing = Preparing::new(self.notes.clone());
        let report = |model, received, total| preparing.report(model, received, total);
        let engines = self
            .speech
            .hearing(&self.agents, self.provider, self.language, &report)
            .await;
        preparing.done();
        let notes = self.notes;
        match engines {
            Ok((vad, stt)) => {
                let listener = Listener::new(vad, stt);
                std::thread::spawn(move || {
                    let hear = |heard| notes.send(Note::Heard(slot, heard)).is_ok();
                    speech::listen(listener, frames, hear);
                });
            }
            Err(error) => {
                tracing::warn!(?error, "meeting listener unavailable");
                let _ = notes.send(Note::Heard(slot, Heard::Failed(error)));
            }
        }
    }
}
