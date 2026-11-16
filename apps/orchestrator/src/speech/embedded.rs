use super::models::{KOKORO, LESSAC, RYAN, SILERO, WHISPER, ZIPFORMER};
use super::pipeline::{Activity, Vad};
use super::{Audio, Language, SAMPLE_RATE, Speech, SpeechError, Stt, Tts, Voice};
use sherpa_onnx::{
    GenerationConfig, OfflineRecognizer, OfflineRecognizerConfig, OfflineTts, OfflineTtsConfig,
    OnlineRecognizer, OnlineRecognizerConfig, OnlineStream, SileroVadModelConfig, VadModelConfig,
    VoiceActivityDetector,
};
use std::{path::Path, sync::Arc};
use tokio::sync::OnceCell;

pub(super) const WINDOW: usize = 512;
const SILENCE_SECONDS: f32 = 0.6;
const SPEECH_SECONDS: f32 = 0.25;
const TAIL_SECONDS: f32 = 0.66;
const KOKORO_FEMALE: i32 = 42;
const KOKORO_MALE: i32 = 43;
const KOKORO_THREADS: i32 = 4;

#[derive(Default)]
pub(super) struct Loaded {
    recognizer: OnceCell<Arc<OnlineRecognizer>>,
    whisper: OnceCell<Arc<OfflineRecognizer>>,
    female: OnceCell<Arc<OfflineTts>>,
    male: OnceCell<Arc<OfflineTts>>,
    kokoro: OnceCell<Arc<OfflineTts>>,
}

fn path(directory: &Path, file: &str) -> Option<String> {
    Some(directory.join(file).to_string_lossy().into_owned())
}

type Progress<'a> = &'a (dyn Fn(&'static str, u64, u64) + Sync);

impl Speech {
    pub(super) async fn silero(&self, progress: Progress<'_>) -> Result<Box<dyn Vad>, SpeechError> {
        let directory = self.models()?.ensure(&SILERO, progress).await?;
        let silero = tokio::task::spawn_blocking(move || Silero::load(&directory))
            .await
            .map_err(std::io::Error::other)??;
        Ok(Box::new(silero))
    }

    pub(super) async fn listening(
        &self,
        language: Language,
        progress: Progress<'_>,
    ) -> Result<Box<dyn Stt>, SpeechError> {
        match language {
            Language::English => self.zipformer(progress).await,
            Language::Portuguese => self.whisper(progress).await,
        }
    }

    pub(super) async fn speaking(
        &self,
        language: Language,
        voice: Voice,
        progress: Progress<'_>,
    ) -> Result<Box<dyn Tts>, SpeechError> {
        match language {
            Language::English => self.piper(voice, progress).await,
            Language::Portuguese => self.kokoro(voice, progress).await,
        }
    }

    async fn whisper(&self, progress: Progress<'_>) -> Result<Box<dyn Stt>, SpeechError> {
        let directory = self.models()?.ensure(&WHISPER, progress).await?;
        let recognizer = self
            .loaded
            .whisper
            .get_or_try_init(|| async {
                tokio::task::spawn_blocking(move || whisper(&directory, "pt"))
                    .await
                    .map_err(std::io::Error::other)?
            })
            .await?;
        Ok(Box::new(Whisper::new(recognizer.clone())))
    }

    async fn kokoro(
        &self,
        voice: Voice,
        progress: Progress<'_>,
    ) -> Result<Box<dyn Tts>, SpeechError> {
        let directory = self.models()?.ensure(&KOKORO, progress).await?;
        let tts = self
            .loaded
            .kokoro
            .get_or_try_init(|| async {
                tokio::task::spawn_blocking(move || kokoro(&directory, "pt-br"))
                    .await
                    .map_err(std::io::Error::other)?
            })
            .await?;
        let speaker = match voice {
            Voice::Female => KOKORO_FEMALE,
            Voice::Male => KOKORO_MALE,
        };
        Ok(Box::new(Kokoro(tts.clone(), speaker)))
    }

    async fn zipformer(&self, progress: Progress<'_>) -> Result<Box<dyn Stt>, SpeechError> {
        let directory = self.models()?.ensure(&ZIPFORMER, progress).await?;
        let recognizer = self
            .loaded
            .recognizer
            .get_or_try_init(|| async {
                tokio::task::spawn_blocking(move || recognizer(&directory))
                    .await
                    .map_err(std::io::Error::other)?
            })
            .await?;
        Ok(Box::new(Zipformer::new(recognizer.clone())))
    }

    async fn piper(
        &self,
        voice: Voice,
        progress: Progress<'_>,
    ) -> Result<Box<dyn Tts>, SpeechError> {
        let (cell, asset) = match voice {
            Voice::Female => (&self.loaded.female, &LESSAC),
            Voice::Male => (&self.loaded.male, &RYAN),
        };
        let directory = self.models()?.ensure(asset, progress).await?;
        let file = asset.file;
        let tts = cell
            .get_or_try_init(|| async {
                tokio::task::spawn_blocking(move || piper(&directory, file))
                    .await
                    .map_err(std::io::Error::other)?
            })
            .await?;
        Ok(Box::new(Piper(tts.clone())))
    }
}

pub fn recognizer(directory: &Path) -> Result<Arc<OnlineRecognizer>, SpeechError> {
    let mut config = OnlineRecognizerConfig::default();
    let stem = "epoch-99-avg-1-chunk-16-left-128.int8.onnx";
    let transducer = &mut config.model_config.transducer;
    transducer.encoder = path(directory, &format!("encoder-{stem}"));
    transducer.decoder = path(directory, &format!("decoder-{stem}"));
    transducer.joiner = path(directory, &format!("joiner-{stem}"));
    config.model_config.tokens = path(directory, "tokens.txt");
    config.model_config.num_threads = 1;
    config.decoding_method = Some("greedy_search".to_owned());
    OnlineRecognizer::create(&config)
        .map(Arc::new)
        .ok_or(SpeechError::Model(ZIPFORMER.name))
}

pub fn piper(directory: &Path, file: &'static str) -> Result<Arc<OfflineTts>, SpeechError> {
    let mut config = OfflineTtsConfig::default();
    let vits = &mut config.model.vits;
    vits.model = path(directory, file);
    vits.tokens = path(directory, "tokens.txt");
    vits.data_dir = path(directory, "espeak-ng-data");
    config.model.num_threads = 1;
    OfflineTts::create(&config)
        .map(Arc::new)
        .ok_or(SpeechError::Model(file))
}

pub fn whisper(directory: &Path, language: &str) -> Result<Arc<OfflineRecognizer>, SpeechError> {
    let mut config = OfflineRecognizerConfig::default();
    let size = WHISPER.file;
    let model = &mut config.model_config.whisper;
    model.encoder = path(directory, &format!("{size}-encoder.int8.onnx"));
    model.decoder = path(directory, &format!("{size}-decoder.int8.onnx"));
    model.language = Some(language.to_owned());
    model.task = Some("transcribe".to_owned());
    model.tail_paddings = -1;
    config.model_config.tokens = path(directory, &format!("{size}-tokens.txt"));
    config.model_config.num_threads = 2;
    OfflineRecognizer::create(&config)
        .map(Arc::new)
        .ok_or(SpeechError::Model(WHISPER.name))
}

pub fn kokoro(directory: &Path, language: &str) -> Result<Arc<OfflineTts>, SpeechError> {
    let mut config = OfflineTtsConfig::default();
    let model = &mut config.model.kokoro;
    model.model = path(directory, KOKORO.file);
    model.voices = path(directory, "voices.bin");
    model.tokens = path(directory, "tokens.txt");
    model.data_dir = path(directory, "espeak-ng-data");
    model.lang = Some(language.to_owned());
    config.model.num_threads = KOKORO_THREADS;
    OfflineTts::create(&config)
        .map(Arc::new)
        .ok_or(SpeechError::Model(KOKORO.name))
}

pub struct Silero(VoiceActivityDetector);

impl Silero {
    pub fn load(directory: &Path) -> Result<Self, SpeechError> {
        let config = VadModelConfig {
            silero_vad: SileroVadModelConfig {
                model: path(directory, SILERO.file),
                threshold: 0.5,
                min_silence_duration: SILENCE_SECONDS,
                min_speech_duration: SPEECH_SECONDS,
                window_size: WINDOW as i32,
                max_speech_duration: 30.0,
            },
            sample_rate: SAMPLE_RATE as i32,
            num_threads: 1,
            ..VadModelConfig::default()
        };
        VoiceActivityDetector::create(&config, 60.0)
            .map(Self)
            .ok_or(SpeechError::Model(SILERO.name))
    }
}

impl Vad for Silero {
    fn accept(&mut self, window: &[f32]) -> Activity {
        self.0.accept_waveform(window);
        let ended = !self.0.is_empty();
        while !self.0.is_empty() {
            self.0.pop();
        }
        Activity {
            speaking: self.0.detected(),
            ended,
        }
    }
}

pub struct Zipformer {
    recognizer: Arc<OnlineRecognizer>,
    stream: OnlineStream,
    last: String,
}

impl Zipformer {
    pub fn new(recognizer: Arc<OnlineRecognizer>) -> Self {
        let stream = recognizer.create_stream();
        Self {
            recognizer,
            stream,
            last: String::new(),
        }
    }

    fn text(&self) -> String {
        while self.recognizer.is_ready(&self.stream) {
            self.recognizer.decode(&self.stream);
        }
        let result = self.recognizer.get_result(&self.stream);
        result.map_or_else(String::new, |result| readable(&result.text))
    }
}

fn readable(text: &str) -> String {
    let lower = text.trim().to_lowercase();
    let mut letters = lower.chars();
    letters.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(letters).collect()
    })
}

impl Stt for Zipformer {
    fn accept(&mut self, samples: &[f32]) -> Result<Option<String>, SpeechError> {
        self.stream.accept_waveform(SAMPLE_RATE as i32, samples);
        let text = self.text();
        if text.is_empty() || text == self.last {
            return Ok(None);
        }
        self.last.clone_from(&text);
        Ok(Some(text))
    }

    fn finish(&mut self) -> Result<String, SpeechError> {
        let tail = vec![0.0; (SAMPLE_RATE as f32 * TAIL_SECONDS) as usize];
        self.stream.accept_waveform(SAMPLE_RATE as i32, &tail);
        self.stream.input_finished();
        let text = self.text();
        self.stream = self.recognizer.create_stream();
        self.last.clear();
        Ok(text)
    }
}

/// Whisper hears a whole utterance at once, so it offers no partial captions.
pub struct Whisper {
    recognizer: Arc<OfflineRecognizer>,
    samples: Vec<f32>,
}

impl Whisper {
    pub fn new(recognizer: Arc<OfflineRecognizer>) -> Self {
        Self {
            recognizer,
            samples: Vec::new(),
        }
    }
}

impl Stt for Whisper {
    fn accept(&mut self, samples: &[f32]) -> Result<Option<String>, SpeechError> {
        self.samples.extend_from_slice(samples);
        Ok(None)
    }

    fn finish(&mut self) -> Result<String, SpeechError> {
        let stream = self.recognizer.create_stream();
        stream.accept_waveform(SAMPLE_RATE as i32, &std::mem::take(&mut self.samples));
        self.recognizer.decode(&stream);
        let text = stream.get_result().map(|result| result.text);
        Ok(text
            .filter(|text| !looped(text))
            .unwrap_or_default()
            .trim()
            .to_owned())
    }
}

/// Whisper sometimes repeats one short phrase until it runs out of tokens instead of transcribing.
fn looped(text: &str) -> bool {
    let words: Vec<String> = text
        .split_whitespace()
        .map(|word| {
            word.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .collect();
    let distinct: std::collections::HashSet<&String> = words.iter().collect();
    words.len() >= 12 && distinct.len() * 4 < words.len()
}

pub struct Piper(pub Arc<OfflineTts>);

impl Tts for Piper {
    fn speak(&self, text: &str, sink: &mut dyn FnMut(Audio) -> bool) -> Result<(), SpeechError> {
        spoken(&self.0, text, 0, sink)
    }
}

pub struct Kokoro(pub Arc<OfflineTts>, pub i32);

impl Tts for Kokoro {
    fn speak(&self, text: &str, sink: &mut dyn FnMut(Audio) -> bool) -> Result<(), SpeechError> {
        spoken(&self.0, text, self.1, sink)
    }
}

fn spoken(
    tts: &OfflineTts,
    text: &str,
    sid: i32,
    sink: &mut dyn FnMut(Audio) -> bool,
) -> Result<(), SpeechError> {
    let progress: Option<fn(&[f32], f32) -> bool> = None;
    let config = GenerationConfig {
        sid,
        ..GenerationConfig::default()
    };
    let generated = tts
        .generate_with_config(text, &config, progress)
        .ok_or(SpeechError::Model("voice"))?;
    let rate = generated.sample_rate() as u32;
    for chunk in generated.samples().chunks(rate as usize / 4) {
        let pcm = chunk
            .iter()
            .flat_map(|sample| ((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes())
            .collect();
        if !sink(Audio { rate, pcm }) {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{looped, readable};

    #[test]
    fn a_looping_transcript_is_dropped_and_real_speech_is_kept() {
        assert!(looped(
            "Que o que é que é que é que é que é que é que é que é que é"
        ));
        assert!(!looped(
            "Quais são as novidades interessantes desta semana nos projetos?"
        ));
        assert!(!looped(
            "Não, não, não, eu disse que a reunião é amanhã às dez horas da manhã."
        ));
        assert!(!looped("sim sim sim"));
    }

    #[test]
    fn a_transcript_reads_as_a_sentence() {
        assert_eq!(readable(" WHAT IS THE TIME"), "What is the time");
        assert_eq!(readable(""), "");
    }
}
