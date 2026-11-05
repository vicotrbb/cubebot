//! Voice calls (docs/adr/console.md#voice-calls-over-the-chat-pipeline): speech to text and back,
//! behind one STT and one TTS trait. Agents never see audio; a finished sentence becomes an
//! ordinary chat turn.
mod embedded;
mod external;
mod models;
mod pipeline;
mod session;
pub(crate) mod settings;

pub use embedded::{Piper, Silero, Zipformer, piper, recognizer};
pub use models::{Asset, LESSAC, Models, RYAN, SILERO, ZIPFORMER};
pub use pipeline::{Activity, Heard, Listener, Sentences, Vad};
pub use session::Session;
pub use settings::{AgentSpeech, Route, SaveSpeech, SpeechSettings};

use crate::connections::Connections;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// All speech enters the pipeline at this rate.
pub const SAMPLE_RATE: u32 = 16_000;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum Provider {
    Embedded,
    Openai,
    Elevenlabs,
}

impl Provider {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Embedded => "embedded",
            Self::Openai => "openai",
            Self::Elevenlabs => "elevenlabs",
        }
    }
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type, utoipa::ToSchema,
)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "text", rename_all = "snake_case")]
pub enum Voice {
    Male,
    Female,
}

impl Voice {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Male => "male",
            Self::Female => "female",
        }
    }
}

/// Speech a provider produced: little-endian PCM16 at `rate`, as the page plays it.
pub struct Audio {
    pub rate: u32,
    pub pcm: Vec<u8>,
}

/// One stream of one person's speech, fed from its own thread.
pub trait Stt: Send {
    /// Takes samples of the current sentence; returns a partial transcript when it changed.
    fn accept(&mut self, samples: &[f32]) -> Result<Option<String>, SpeechError>;
    /// Ends the sentence, returns its transcript and readies the stream for the next one.
    fn finish(&mut self) -> Result<String, SpeechError>;
}

/// One voice, speaking from its own thread.
pub trait Tts: Send {
    /// Sends `text` to `sink` as audio until `sink` returns false.
    fn speak(&self, text: &str, sink: &mut dyn FnMut(Audio) -> bool) -> Result<(), SpeechError>;
}

#[derive(Debug)]
pub enum SpeechError {
    /// No data directory to hold the embedded models.
    NoDirectory,
    Io(std::io::Error),
    Http(reqwest::Error),
    /// A provider refused the request; the status, never the body, which may echo the key.
    Refused(u16),
    Checksum(&'static str),
    Model(&'static str),
    /// The provider needs a key this agent cannot use.
    NoKey(Provider),
}

impl std::fmt::Display for SpeechError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoDirectory => {
                formatter.write_str("this installation has no data directory for speech models")
            }
            Self::Io(_) => formatter.write_str("the speech models could not be stored"),
            Self::Http(_) => formatter.write_str("the speech provider could not be reached"),
            Self::Refused(status) => {
                write!(
                    formatter,
                    "the speech provider refused the request ({status})"
                )
            }
            Self::Checksum(name) => {
                write!(
                    formatter,
                    "the download of {name} failed its integrity check"
                )
            }
            Self::Model(name) => write!(formatter, "the speech model {name} could not be loaded"),
            Self::NoKey(provider) => {
                write!(formatter, "no usable key for {} speech", provider.as_str())
            }
        }
    }
}

impl std::error::Error for SpeechError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Http(error) => Some(error),
            _ => None,
        }
    }
}

impl From<std::io::Error> for SpeechError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error)
    }
}

impl From<reqwest::Error> for SpeechError {
    fn from(error: reqwest::Error) -> Self {
        Self::Http(error)
    }
}

/// What one call speaks and listens with.
pub struct Engines {
    pub vad: Box<dyn Vad>,
    pub stt: Box<dyn Stt>,
    pub tts: Box<dyn Tts>,
}

/// Builds a test's engines in place of real providers.
pub type StandIn = Arc<dyn Fn() -> Engines + Send + Sync>;

/// The installation's speech providers and the embedded models it has loaded.
#[derive(Clone)]
pub struct Speech {
    models: Option<Arc<Models>>,
    loaded: Arc<embedded::Loaded>,
    connections: Option<Connections>,
    stand_in: Option<StandIn>,
    endpoints: external::Endpoints,
}

impl Speech {
    /// Models live under the shared-file directory, beside the blobs its reconciler owns.
    pub fn environment(connections: Option<Connections>) -> Self {
        let models = std::env::var_os("CUBEBOT_STORAGE_DIRECTORY").map(|root| {
            Arc::new(Models::new(
                std::path::Path::new(&root).join("speech-models"),
            ))
        });
        Self {
            models,
            loaded: Arc::default(),
            connections,
            stand_in: None,
            endpoints: external::Endpoints::default(),
        }
    }

    /// Calls run on `stand_in` instead of real providers. Proves nothing about speech.
    pub fn stand_in(stand_in: StandIn) -> Self {
        Self {
            models: None,
            loaded: Arc::default(),
            connections: None,
            stand_in: Some(stand_in),
            endpoints: external::Endpoints::default(),
        }
    }

    /// Readies a call's engines, downloading what it needs on first use.
    #[tracing::instrument(skip_all, fields(agent = %agent.0))]
    pub(crate) async fn engines(
        &self,
        agent: crate::agents::AgentId,
        route: Route,
        progress: &(dyn Fn(&'static str, u64, u64) + Sync),
    ) -> Result<Engines, SpeechError> {
        if let Some(stand_in) = &self.stand_in {
            return Ok(stand_in());
        }
        let vad = self.silero(progress).await?;
        let stt: Box<dyn Stt> = match route.stt {
            Provider::Embedded => self.zipformer(progress).await?,
            provider => Box::new(
                self.endpoints
                    .listener(provider, self.key(agent, provider).await?),
            ),
        };
        let tts: Box<dyn Tts> = match route.tts {
            Provider::Embedded => self.piper(route.voice, progress).await?,
            provider => Box::new(self.endpoints.speaker(
                provider,
                self.key(agent, provider).await?,
                route.voice,
            )),
        };
        Ok(Engines { vad, stt, tts })
    }

    fn models(&self) -> Result<&Models, SpeechError> {
        self.models.as_deref().ok_or(SpeechError::NoDirectory)
    }

    async fn key(
        &self,
        agent: crate::agents::AgentId,
        provider: Provider,
    ) -> Result<String, SpeechError> {
        let connections = self
            .connections
            .as_ref()
            .ok_or(SpeechError::NoKey(provider))?;
        connections
            .speech_key(agent, provider)
            .await
            .map_err(|error| {
                tracing::warn!(
                    ?error,
                    provider = provider.as_str(),
                    "speech key unavailable"
                );
                SpeechError::NoKey(provider)
            })
    }
}
