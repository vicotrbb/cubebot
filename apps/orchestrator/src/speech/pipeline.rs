//! Turn-taking and sentence splitting: what a call does, whichever providers it uses.
use super::{Audio, SpeechError, Stt, Tts};
use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use tokio::sync::mpsc;

/// Audio kept before speech is detected, so a sentence's first sound reaches the STT.
const PREROLL: usize = 8_000;
const WINDOW: usize = super::embedded::WINDOW;
/// A sentence is spoken once it is this long even without punctuation, so a long one cannot
/// hold back the first audio.
const LONGEST_SENTENCE: usize = 280;

/// What the voice activity detector said about one window.
pub struct Activity {
    pub speaking: bool,
    /// A stretch of speech just ended.
    pub ended: bool,
}

/// Endpointing, behind a seam so turn-taking can be tested without a model.
pub trait Vad: Send {
    fn accept(&mut self, window: &[f32]) -> Activity;
}

#[derive(Debug)]
pub enum Heard {
    /// The person started talking; any playback must stop.
    Speech,
    Partial(String),
    /// A finished sentence.
    Utterance(String),
    Failed(SpeechError),
}

/// Follows one person's audio from the first sample of a call to the last.
pub struct Listener {
    vad: Box<dyn Vad>,
    stt: Box<dyn Stt>,
    pending: Vec<f32>,
    preroll: VecDeque<f32>,
    speaking: bool,
}

impl Listener {
    pub fn new(vad: Box<dyn Vad>, stt: Box<dyn Stt>) -> Self {
        Self {
            vad,
            stt,
            pending: Vec::with_capacity(WINDOW * 2),
            preroll: VecDeque::with_capacity(PREROLL + WINDOW),
            speaking: false,
        }
    }

    pub fn hear(&mut self, samples: &[f32]) -> Vec<Heard> {
        let mut heard = Vec::new();
        self.pending.extend_from_slice(samples);
        let windows = self.pending.len() / WINDOW;
        let ready: Vec<f32> = self.pending.drain(..windows * WINDOW).collect();
        for window in ready.chunks(WINDOW) {
            if let Err(error) = self.window(window, &mut heard) {
                heard.push(Heard::Failed(error));
                self.speaking = false;
            }
        }
        heard
    }

    fn window(&mut self, window: &[f32], heard: &mut Vec<Heard>) -> Result<(), SpeechError> {
        let activity = self.vad.accept(window);
        if !self.speaking && activity.speaking {
            self.speaking = true;
            heard.push(Heard::Speech);
            let earlier: Vec<f32> = self.preroll.drain(..).collect();
            self.feed(&earlier, heard)?;
        }
        if self.speaking {
            self.feed(window, heard)?;
        } else {
            self.preroll.extend(window);
            let excess = self.preroll.len().saturating_sub(PREROLL);
            self.preroll.drain(..excess);
        }
        if activity.ended && self.speaking {
            self.speaking = false;
            let text = self.stt.finish()?;
            if !text.is_empty() {
                heard.push(Heard::Utterance(text));
            }
        }
        Ok(())
    }

    fn feed(&mut self, samples: &[f32], heard: &mut Vec<Heard>) -> Result<(), SpeechError> {
        if let Some(partial) = self.stt.accept(samples)? {
            heard.push(Heard::Partial(partial));
        }
        Ok(())
    }
}

/// The listening thread; it ends when the call drops its audio sender.
pub(super) fn listen(
    mut listener: Listener,
    mut audio: mpsc::Receiver<Vec<f32>>,
    out: mpsc::UnboundedSender<Heard>,
) {
    while let Some(samples) = audio.blocking_recv() {
        for heard in listener.hear(&samples) {
            if out.send(heard).is_err() {
                return;
            }
        }
    }
}

/// The speaking thread. `epoch` moves on when the person interrupts, which skips sentences queued
/// before that and stops the one being spoken.
pub(super) fn speak(
    tts: Box<dyn Tts>,
    mut sentences: mpsc::UnboundedReceiver<(u64, String)>,
    epoch: Arc<AtomicU64>,
    audio: mpsc::Sender<Audio>,
    failures: mpsc::UnboundedSender<Heard>,
) {
    while let Some((said_in, sentence)) = sentences.blocking_recv() {
        if said_in != epoch.load(Ordering::SeqCst) {
            continue;
        }
        let spoken = tts.speak(&sentence, &mut |chunk| {
            said_in == epoch.load(Ordering::SeqCst) && audio.blocking_send(chunk).is_ok()
        });
        if let Err(error) = spoken
            && failures.send(Heard::Failed(error)).is_err()
        {
            return;
        }
    }
}

/// Cuts a streamed reply into sentences fit to say aloud. Code is not read out.
#[derive(Default)]
pub struct Sentences {
    buffer: String,
    fenced: bool,
}

impl Sentences {
    pub fn push(&mut self, delta: &str) -> Vec<String> {
        self.buffer.push_str(delta);
        let mut sentences = Vec::new();
        while let Some(end) = self.boundary() {
            let piece: String = self.buffer.drain(..end).collect();
            self.collect(&piece, &mut sentences);
        }
        sentences
    }

    /// Whatever remains when the reply ends.
    pub fn finish(&mut self) -> Vec<String> {
        let rest = std::mem::take(&mut self.buffer);
        let mut sentences = Vec::new();
        self.collect(&rest, &mut sentences);
        sentences
    }

    fn collect(&mut self, piece: &str, sentences: &mut Vec<String>) {
        let piece = piece.trim();
        if piece.starts_with("```") {
            self.fenced = !self.fenced;
        } else if !self.fenced {
            let said = plain(piece);
            if said.chars().any(char::is_alphanumeric) {
                sentences.push(said);
            }
        }
    }

    /// The byte offset after the next complete piece: a line, or a sentence outside a fence.
    fn boundary(&self) -> Option<usize> {
        let mut characters = self.buffer.char_indices().peekable();
        while let Some((at, character)) = characters.next() {
            if character == '\n' {
                return Some(at + 1);
            }
            let next = characters.peek().map(|(_, next)| *next);
            if !self.fenced
                && matches!(character, '.' | '!' | '?')
                && next.is_some_and(char::is_whitespace)
            {
                return Some(at + character.len_utf8());
            }
        }
        (!self.fenced && self.buffer.len() > LONGEST_SENTENCE)
            .then(|| {
                let head = &self.buffer[..self.buffer.floor_char_boundary(LONGEST_SENTENCE)];
                head.rfind(' ').unwrap_or(head.len())
            })
            .filter(|end| *end > 0)
    }
}

/// Markdown as it should sound: no markup, and a link is its label.
fn plain(line: &str) -> String {
    let line = line.trim_start_matches(['#', '>', ' ']);
    let line = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .unwrap_or(line);
    let mut out = String::with_capacity(line.len());
    let mut rest = line;
    while let Some(open) = rest.find('[') {
        let Some((label, target)) = rest[open + 1..].split_once("](") else {
            break;
        };
        let Some(close) = target.find(')') else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push_str(label);
        rest = &target[close + 1..];
    }
    out.push_str(rest);
    out.retain(|character| !matches!(character, '*' | '`' | '|'));
    out.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Speech while the samples are loud, ending at the first quiet window after it.
    struct Loud(bool);
    impl Vad for Loud {
        fn accept(&mut self, window: &[f32]) -> Activity {
            let speaking = window[0] > 0.0;
            let ended = self.0 && !speaking;
            self.0 = speaking;
            Activity { speaking, ended }
        }
    }

    #[derive(Default)]
    struct Script {
        fed: usize,
        finals: VecDeque<Result<String, SpeechError>>,
    }
    impl Stt for Script {
        fn accept(&mut self, samples: &[f32]) -> Result<Option<String>, SpeechError> {
            self.fed += samples.len();
            Ok(Some(format!("{} heard", self.fed)))
        }
        fn finish(&mut self) -> Result<String, SpeechError> {
            self.finals.pop_front().unwrap()
        }
    }

    fn listener(finals: Vec<Result<String, SpeechError>>) -> Listener {
        let stt = Script {
            fed: 0,
            finals: finals.into(),
        };
        Listener::new(Box::new(Loud(false)), Box::new(stt))
    }

    #[test]
    fn speech_interrupts_then_gives_partials_and_one_utterance() {
        let mut listener = listener(vec![Ok("hello".to_owned())]);
        // Quiet first: it becomes the preroll, not a turn.
        assert!(listener.hear(&[0.0; WINDOW]).is_empty());
        let heard = listener.hear(&[0.5; WINDOW]);
        assert!(matches!(heard[0], Heard::Speech));
        assert!(matches!(&heard[1], Heard::Partial(text) if text == &format!("{WINDOW} heard")));
        // The window that started speech follows its preroll into the STT.
        assert!(
            matches!(&heard[2], Heard::Partial(text) if text == &format!("{} heard", 2 * WINDOW))
        );
        let heard = listener.hear(&[0.0; WINDOW]);
        assert!(matches!(heard.last(), Some(Heard::Utterance(text)) if text == "hello"));
    }

    #[test]
    fn silence_alone_and_an_empty_transcript_take_no_turn() {
        let mut listener = listener(vec![Ok(String::new())]);
        assert!(listener.hear(&[0.0; WINDOW * 40]).is_empty());
        listener.hear(&[0.5; WINDOW]);
        let heard = listener.hear(&[0.0; WINDOW]);
        assert!(!heard.iter().any(|item| matches!(item, Heard::Utterance(_))));
    }

    #[test]
    fn frames_of_any_size_are_cut_into_windows() {
        let mut listener = listener(vec![]);
        assert!(listener.hear(&[0.5; WINDOW - 1]).is_empty());
        assert!(!listener.hear(&[0.5; 1]).is_empty());
    }

    #[test]
    fn a_failed_transcript_is_reported_and_the_call_keeps_listening() {
        let mut listener = listener(vec![Err(SpeechError::Refused(500)), Ok("next".to_owned())]);
        listener.hear(&[0.5; WINDOW]);
        let heard = listener.hear(&[0.0; WINDOW]);
        assert!(matches!(
            heard.last(),
            Some(Heard::Failed(SpeechError::Refused(500)))
        ));
        listener.hear(&[0.5; WINDOW]);
        let heard = listener.hear(&[0.0; WINDOW]);
        assert!(matches!(heard.last(), Some(Heard::Utterance(text)) if text == "next"));
    }

    #[test]
    fn replies_split_as_they_stream() {
        let mut sentences = Sentences::default();
        assert!(sentences.push("Hello there").is_empty());
        assert_eq!(sentences.push(". It costs 3.5 da"), ["Hello there."]);
        assert_eq!(sentences.push("ys! Next"), ["It costs 3.5 days!"]);
        assert_eq!(sentences.finish(), ["Next"]);
    }

    #[test]
    fn code_is_skipped_and_markup_is_not_read() {
        let mut sentences = Sentences::default();
        let said = sentences.push(
            "## **Plan**\n- see [the docs](https://a.b/c) now\n```rust\nfn main() { x. y }\n```\nDone `it`.\n",
        );
        assert_eq!(said, ["Plan", "see the docs now", "Done it."]);
    }

    #[test]
    fn a_reply_without_punctuation_still_comes_out() {
        let mut sentences = Sentences::default();
        let said = sentences.push(&"word ".repeat(100));
        assert!(!said.is_empty() && said[0].len() <= LONGEST_SENTENCE);
    }

    struct Count(Arc<AtomicU64>);
    impl Tts for Count {
        fn speak(&self, _: &str, sink: &mut dyn FnMut(Audio) -> bool) -> Result<(), SpeechError> {
            // The person interrupts after each sentence's first chunk.
            sink(Audio {
                rate: 1,
                pcm: vec![0],
            });
            self.0.fetch_add(1, Ordering::SeqCst);
            sink(Audio {
                rate: 1,
                pcm: vec![1],
            });
            Ok(())
        }
    }

    #[test]
    fn interrupting_stops_this_sentence_and_skips_those_queued() {
        let epoch = Arc::new(AtomicU64::new(0));
        let (sentences, queue) = mpsc::unbounded_channel();
        let (audio, mut chunks) = mpsc::channel(8);
        let (failures, _kept) = mpsc::unbounded_channel();
        sentences.send((0, "one".to_owned())).unwrap();
        sentences.send((0, "two".to_owned())).unwrap();
        sentences.send((1, "after".to_owned())).unwrap();
        drop(sentences);
        speak(
            Box::new(Count(epoch.clone())),
            queue,
            epoch,
            audio,
            failures,
        );
        let mut heard = Vec::new();
        while let Ok(chunk) = chunks.try_recv() {
            heard.push(chunk.pcm[0]);
        }
        // "one" stops after its first chunk, "two" never starts, and "after", said in the new
        // epoch, starts and is interrupted in turn.
        assert_eq!(heard, [0, 0]);
    }
}
