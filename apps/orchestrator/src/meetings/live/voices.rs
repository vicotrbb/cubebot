use super::Handle;
use super::attendants::Note;
use super::wire::Server;
use crate::speech::{Audio, Tts};
use std::panic::AssertUnwindSafe;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::time::Instant;
use uuid::Uuid;

const PIECE: Duration = Duration::from_millis(100);
const LEAD: Duration = Duration::from_millis(300);
const SOUNDS: usize = 32;
const STOPPED: &str = "the speech engine stopped unexpectedly";

enum Line {
    Say(u64, String),
    End(Uuid),
}

enum Sound {
    Audio(u64, Audio),
    Failed(u64, String),
    End(Uuid),
}

type Parked = (mpsc::UnboundedReceiver<Line>, mpsc::Sender<Sound>);

pub(super) struct Voice {
    lines: mpsc::UnboundedSender<Line>,
    epoch: Arc<AtomicU64>,
    parked: Option<Parked>,
}

impl Voice {
    pub(super) fn new(
        room: Handle,
        slot: u16,
        name: String,
        spoken: mpsc::UnboundedSender<Note>,
    ) -> Self {
        let (lines, waiting) = mpsc::unbounded_channel();
        let (sounds, heard) = mpsc::channel(SOUNDS);
        let epoch = Arc::new(AtomicU64::new(0));
        let stage = Stage {
            room,
            slot,
            name,
            epoch: epoch.clone(),
            clock: Instant::now(),
            failed: None,
        };
        tokio::spawn(stage.pace(heard, spoken));
        Self {
            lines,
            epoch,
            parked: Some((waiting, sounds)),
        }
    }

    pub(super) fn ready(&mut self, tts: Option<Box<dyn Tts>>) {
        if let Some((lines, sounds)) = self.parked.take() {
            let epoch = self.epoch.clone();
            std::thread::spawn(move || speak(tts, lines, &epoch, &sounds));
        }
    }

    pub(super) fn say(&self, sentence: String) {
        let epoch = self.epoch.load(Ordering::SeqCst);
        let _ = self.lines.send(Line::Say(epoch, sentence));
    }

    pub(super) fn end(&self, turn: Uuid) {
        let _ = self.lines.send(Line::End(turn));
    }

    pub(super) fn interrupt(&self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
    }
}

fn speak(
    tts: Option<Box<dyn Tts>>,
    mut lines: mpsc::UnboundedReceiver<Line>,
    epoch: &AtomicU64,
    sounds: &mpsc::Sender<Sound>,
) {
    while let Some(line) = lines.blocking_recv() {
        let open = match line {
            Line::Say(said_in, sentence) => {
                let current = said_in == epoch.load(Ordering::SeqCst);
                if let Some(tts) = tts.as_ref().filter(|_| current)
                    && let Err(reason) = voiced(tts.as_ref(), &sentence, said_in, epoch, sounds)
                {
                    let _ = sounds.blocking_send(Sound::Failed(said_in, reason));
                }
                !sounds.is_closed()
            }
            Line::End(turn) => sounds.blocking_send(Sound::End(turn)).is_ok(),
        };
        if !open {
            return;
        }
    }
}

fn voiced(
    tts: &dyn Tts,
    sentence: &str,
    said_in: u64,
    epoch: &AtomicU64,
    sounds: &mpsc::Sender<Sound>,
) -> Result<(), String> {
    let spoken = std::panic::catch_unwind(AssertUnwindSafe(|| {
        tts.speak(sentence, &mut |audio| {
            said_in == epoch.load(Ordering::SeqCst)
                && sounds.blocking_send(Sound::Audio(said_in, audio)).is_ok()
        })
    }));
    let Ok(spoken) = spoken else {
        return Err(STOPPED.to_owned());
    };
    spoken.map_err(|error| {
        tracing::warn!(?error, "meeting speech failed");
        error.to_string()
    })
}

struct Stage {
    room: Handle,
    slot: u16,
    name: String,
    epoch: Arc<AtomicU64>,
    clock: Instant,
    failed: Option<u64>,
}

impl Stage {
    async fn pace(
        mut self,
        mut sounds: mpsc::Receiver<Sound>,
        spoken: mpsc::UnboundedSender<Note>,
    ) {
        while let Some(sound) = sounds.recv().await {
            match sound {
                Sound::Audio(said_in, audio) => self.play(said_in, &audio).await,
                Sound::Failed(said_in, reason) => self.failed(said_in, reason).await,
                Sound::End(turn) => {
                    tokio::time::sleep_until(self.clock).await;
                    if spoken.send(Note::Spoken(turn)).is_err() {
                        return;
                    }
                }
            }
        }
    }

    async fn failed(&mut self, said_in: u64, reason: String) {
        if self.failed == Some(said_in) {
            return;
        }
        self.failed = Some(said_in);
        let text = format!("{} could not speak: {reason}", self.name);
        self.room.announce(Server::Notice { text }, None).await;
    }

    fn current(&mut self, said_in: u64) -> bool {
        let current = said_in == self.epoch.load(Ordering::SeqCst);
        if !current {
            self.clock = Instant::now();
        }
        current
    }

    async fn play(&mut self, said_in: u64, audio: &Audio) {
        for piece in audio.pcm.chunks(piece_bytes(audio.rate)) {
            if let Some(wait) = wait(self.clock, Instant::now()) {
                tokio::time::sleep(wait).await;
            }
            if !self.current(said_in) {
                return;
            }
            self.room.agent_audio(self.slot, audio.rate, piece).await;
            self.clock = self.clock.max(Instant::now()) + length(audio.rate, piece.len());
        }
    }
}

fn piece_bytes(rate: u32) -> usize {
    let samples = u128::from(rate) * PIECE.as_millis() / 1000;
    usize::try_from(samples).unwrap_or(usize::MAX).max(1) * 2
}

fn length(rate: u32, bytes: usize) -> Duration {
    if rate == 0 {
        return Duration::ZERO;
    }
    Duration::from_secs_f64(bytes as f64 / 2.0 / f64::from(rate))
}

fn wait(clock: Instant, now: Instant) -> Option<Duration> {
    clock
        .checked_duration_since(now + LEAD)
        .filter(|wait| !wait.is_zero())
}

#[cfg(test)]
mod tests {
    use super::super::room::{Command, Seating};
    use super::*;
    use crate::speech::SpeechError;

    #[test]
    fn agent_audio_goes_out_in_pieces_of_at_most_100_ms() {
        assert_eq!(piece_bytes(22_050), 4_410);
        assert_eq!(piece_bytes(16_000), 3_200);
        assert_eq!(length(22_050, piece_bytes(22_050)), PIECE);
        let second = vec![0_u8; 44_100];
        let pieces: Vec<usize> = second
            .chunks(piece_bytes(22_050))
            .map(<[u8]>::len)
            .collect();
        assert_eq!(pieces, [4_410; 10]);
        assert_eq!(length(0, 100), Duration::ZERO);
    }

    fn stage(epoch: u64) -> (Stage, mpsc::Receiver<Command>) {
        let (commands, sent) = mpsc::channel(64);
        let (invited, _) = mpsc::channel(1);
        let stage = Stage {
            room: Handle { commands, invited },
            slot: 3,
            name: "Cara".to_owned(),
            epoch: Arc::new(AtomicU64::new(epoch)),
            clock: Instant::now(),
            failed: None,
        };
        (stage, sent)
    }

    #[tokio::test(start_paused = true)]
    async fn agent_audio_is_paced_to_real_time_at_most_300_ms_ahead() {
        let (mut stage, mut sent) = stage(0);
        let start = Instant::now();
        let audio = Audio {
            rate: 16_000,
            pcm: vec![0; 64_000],
        };
        tokio::spawn(async move { stage.play(0, &audio).await });
        let mut ahead = Duration::ZERO;
        while let Some(command) = sent.recv().await {
            let Command::Agent(Seating::Audio { slot, frame }) = command else {
                continue;
            };
            assert_eq!((slot, frame.len()), (3, 7 + 3_200));
            let elapsed = start.elapsed();
            assert!(ahead <= elapsed + LEAD, "{ahead:?} ahead at {elapsed:?}");
            assert!(elapsed <= ahead, "{elapsed:?} late for {ahead:?}");
            ahead += PIECE;
        }
        assert_eq!(ahead, Duration::from_secs(2));
    }

    #[tokio::test]
    async fn a_voice_that_fails_tells_the_room_once_per_reply() {
        let (mut stage, mut sent) = stage(0);
        tokio::spawn(async move {
            stage.failed(0, "refused".to_owned()).await;
            stage.failed(0, "refused".to_owned()).await;
            stage.failed(1, "refused again".to_owned()).await;
        });
        let mut notices = Vec::new();
        while let Some(command) = sent.recv().await {
            if let Command::Announce {
                message: Server::Notice { text },
                to: None,
            } = command
            {
                notices.push(text);
            }
        }
        assert_eq!(
            notices,
            [
                "Cara could not speak: refused",
                "Cara could not speak: refused again"
            ]
        );
    }

    struct Broken;

    impl Tts for Broken {
        fn speak(&self, _: &str, _: &mut dyn FnMut(Audio) -> bool) -> Result<(), SpeechError> {
            panic!("engine broke")
        }
    }

    #[tokio::test]
    async fn a_panicking_voice_still_reports_and_ends_its_turn() {
        let (lines, waiting) = mpsc::unbounded_channel();
        let (sounds, mut heard) = mpsc::channel(SOUNDS);
        let epoch = Arc::new(AtomicU64::new(0));
        let turn = Uuid::from_u128(7);
        lines.send(Line::Say(0, "Hello.".to_owned())).unwrap();
        lines.send(Line::Say(0, "Again.".to_owned())).unwrap();
        lines.send(Line::End(turn)).unwrap();
        drop(lines);
        let tts: Option<Box<dyn Tts>> = Some(Box::new(Broken));
        std::thread::spawn(move || speak(tts, waiting, &epoch, &sounds));
        let mut seen = Vec::new();
        while let Some(sound) = heard.recv().await {
            seen.push(match sound {
                Sound::Failed(_, reason) => reason,
                Sound::End(ended) => ended.to_string(),
                Sound::Audio(..) => "audio".to_owned(),
            });
        }
        let failed = STOPPED.to_owned();
        assert_eq!(seen, [failed.clone(), failed, turn.to_string()]);
    }

    #[tokio::test(start_paused = true)]
    async fn interrupted_audio_is_not_sent() {
        let (mut stage, mut sent) = stage(1);
        let audio = Audio {
            rate: 16_000,
            pcm: vec![0; 6_400],
        };
        tokio::spawn(async move { stage.play(0, &audio).await });
        assert!(sent.recv().await.is_none());
    }
}
