use super::models::{LESSAC, RYAN, SILERO, ZIPFORMER};
use super::pipeline::{Activity, Vad};
use super::{Audio, SAMPLE_RATE, Speech, SpeechError, Stt, Tts, Voice};
use sherpa_onnx::{
    GenerationConfig, OfflineTts, OfflineTtsConfig, OnlineRecognizer, OnlineRecognizerConfig,
    OnlineStream, SileroVadModelConfig, VadModelConfig, VoiceActivityDetector,
};
use std::{path::Path, sync::Arc};
use tokio::sync::OnceCell;

pub(super) const WINDOW: usize = 512;
const SILENCE_SECONDS: f32 = 0.6;
const SPEECH_SECONDS: f32 = 0.25;
const TAIL_SECONDS: f32 = 0.66;

#[derive(Default)]
pub(super) struct Loaded {
    recognizer: OnceCell<Arc<OnlineRecognizer>>,
    female: OnceCell<Arc<OfflineTts>>,
    male: OnceCell<Arc<OfflineTts>>,
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

    pub(super) async fn zipformer(
        &self,
        progress: Progress<'_>,
    ) -> Result<Box<dyn Stt>, SpeechError> {
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

    pub(super) async fn piper(
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

pub struct Piper(pub Arc<OfflineTts>);

impl Tts for Piper {
    fn speak(&self, text: &str, sink: &mut dyn FnMut(Audio) -> bool) -> Result<(), SpeechError> {
        let progress: Option<fn(&[f32], f32) -> bool> = None;
        let generated = self
            .0
            .generate_with_config(text, &GenerationConfig::default(), progress)
            .ok_or(SpeechError::Model("piper"))?;
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
}

#[cfg(test)]
mod tests {
    use super::readable;

    #[test]
    fn a_transcript_reads_as_a_sentence() {
        assert_eq!(readable(" WHAT IS THE TIME"), "What is the time");
        assert_eq!(readable(""), "");
    }
}
