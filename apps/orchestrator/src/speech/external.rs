//! OpenAI and ElevenLabs. Silero already found the sentence, so STT uploads it whole and has no
//! partial transcripts. Each thread blocks on the orchestrator's runtime for its own requests.
use super::{Audio, Provider, SAMPLE_RATE, SpeechError, Stt, Tts, Voice};
use futures_util::StreamExt;
use tokio::runtime::Handle;

const OPENAI_STT: &str = "gpt-4o-mini-transcribe";
const OPENAI_TTS: &str = "gpt-4o-mini-tts";
/// OpenAI's `pcm` format.
const OPENAI_RATE: u32 = 24_000;
const ELEVENLABS_STT: &str = "scribe_v1";
const ELEVENLABS_TTS: &str = "eleven_flash_v2_5";

/// Where the providers answer; a test points them at a stand-in.
#[derive(Clone)]
pub struct Endpoints {
    pub openai: String,
    pub elevenlabs: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            openai: "https://api.openai.com".to_owned(),
            elevenlabs: "https://api.elevenlabs.io".to_owned(),
        }
    }
}

impl Endpoints {
    pub(super) fn listener(&self, provider: Provider, key: String) -> Transcriber {
        Transcriber {
            client: reqwest::Client::new(),
            runtime: Handle::current(),
            provider,
            origin: self.origin(provider),
            key,
            samples: Vec::new(),
        }
    }

    pub(super) fn speaker(&self, provider: Provider, key: String, voice: Voice) -> Speaker {
        Speaker {
            client: reqwest::Client::new(),
            runtime: Handle::current(),
            provider,
            origin: self.origin(provider),
            key,
            voice,
        }
    }

    fn origin(&self, provider: Provider) -> String {
        match provider {
            Provider::Elevenlabs => self.elevenlabs.clone(),
            _ => self.openai.clone(),
        }
    }
}

/// A 16 kHz mono PCM16 WAV of `samples`.
fn wav(samples: &[f32]) -> Vec<u8> {
    let bytes = (samples.len() * 2) as u32;
    let mut out = Vec::with_capacity(44 + bytes as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + bytes).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&SAMPLE_RATE.to_le_bytes());
    out.extend_from_slice(&(SAMPLE_RATE * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&bytes.to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&((sample.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
    out
}

/// A multipart/form-data body with one model field and one WAV file, and its boundary.
fn form(field: &str, model: &str, audio: &[u8]) -> (String, Vec<u8>) {
    let boundary = format!("cubebot{}", uuid::Uuid::new_v4().simple());
    let mut body = format!(
        "--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"\r\n\r\n{model}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"speech.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
    )
    .into_bytes();
    body.extend_from_slice(audio);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (boundary, body)
}

pub struct Transcriber {
    client: reqwest::Client,
    runtime: Handle,
    provider: Provider,
    origin: String,
    key: String,
    samples: Vec<f32>,
}

impl Transcriber {
    async fn transcribe(&self) -> Result<String, SpeechError> {
        let audio = wav(&self.samples);
        let request = match self.provider {
            Provider::Elevenlabs => {
                let (boundary, body) = form("model_id", ELEVENLABS_STT, &audio);
                self.client
                    .post(format!("{}/v1/speech-to-text", self.origin))
                    .header("xi-api-key", &self.key)
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(body)
            }
            _ => {
                let (boundary, body) = form("model", OPENAI_STT, &audio);
                self.client
                    .post(format!("{}/v1/audio/transcriptions", self.origin))
                    .bearer_auth(&self.key)
                    .header(
                        "content-type",
                        format!("multipart/form-data; boundary={boundary}"),
                    )
                    .body(body)
            }
        };
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(SpeechError::Refused(response.status().as_u16()));
        }
        let body = response.bytes().await?;
        let parsed: serde_json::Value =
            serde_json::from_slice(&body).map_err(std::io::Error::other)?;
        Ok(parsed["text"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_owned())
    }
}

impl Stt for Transcriber {
    fn accept(&mut self, samples: &[f32]) -> Result<Option<String>, SpeechError> {
        self.samples.extend_from_slice(samples);
        Ok(None)
    }

    fn finish(&mut self) -> Result<String, SpeechError> {
        let text = self.runtime.block_on(self.transcribe());
        self.samples.clear();
        text
    }
}

pub struct Speaker {
    client: reqwest::Client,
    runtime: Handle,
    provider: Provider,
    origin: String,
    key: String,
    voice: Voice,
}

impl Speaker {
    /// The request and the sample rate of what it returns.
    fn request(&self, text: &str) -> (reqwest::RequestBuilder, u32) {
        match self.provider {
            Provider::Elevenlabs => {
                // Rachel and Adam from ElevenLabs' default library.
                let voice = match self.voice {
                    Voice::Female => "21m00Tcm4TlvDq8ikWAM",
                    Voice::Male => "pNInz6obpgDQGcFmaJgB",
                };
                let url = format!(
                    "{}/v1/text-to-speech/{voice}/stream?output_format=pcm_16000",
                    self.origin
                );
                let body = serde_json::json!({ "text": text, "model_id": ELEVENLABS_TTS });
                let request = self.client.post(url).header("xi-api-key", &self.key);
                (request.json(&body), SAMPLE_RATE)
            }
            _ => {
                let voice = match self.voice {
                    Voice::Female => "nova",
                    Voice::Male => "onyx",
                };
                let body = serde_json::json!({
                    "model": OPENAI_TTS, "voice": voice, "input": text, "response_format": "pcm",
                });
                let url = format!("{}/v1/audio/speech", self.origin);
                let request = self.client.post(url).bearer_auth(&self.key);
                (request.json(&body), OPENAI_RATE)
            }
        }
    }

    async fn stream(
        &self,
        text: &str,
        sink: &mut dyn FnMut(Audio) -> bool,
    ) -> Result<(), SpeechError> {
        let (request, rate) = self.request(text);
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(SpeechError::Refused(response.status().as_u16()));
        }
        let mut body = response.bytes_stream();
        // Network chunks can split a sample.
        let mut pcm = Vec::new();
        while let Some(chunk) = body.next().await {
            pcm.extend_from_slice(&chunk?);
            let whole = pcm.len() & !1;
            let rest = pcm.split_off(whole);
            if !pcm.is_empty() && !sink(Audio { rate, pcm }) {
                return Ok(());
            }
            pcm = rest;
        }
        Ok(())
    }
}

impl Tts for Speaker {
    fn speak(&self, text: &str, sink: &mut dyn FnMut(Audio) -> bool) -> Result<(), SpeechError> {
        self.runtime.block_on(self.stream(text, sink))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        body::Bytes,
        http::{HeaderMap, StatusCode, Uri},
        routing::post,
    };
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Seen {
        path: String,
        auth: String,
        body: Vec<u8>,
    }

    type Log = Arc<Mutex<Seen>>;

    fn record(log: &Log, uri: &Uri, headers: &HeaderMap, body: &Bytes) {
        let header = |name: &str| {
            headers
                .get(name)
                .map(|value| value.to_str().unwrap().to_owned())
        };
        *log.lock().unwrap() = Seen {
            path: uri.to_string(),
            auth: header("authorization").or(header("xi-api-key")).unwrap(),
            body: body.to_vec(),
        };
    }

    /// Follows each provider's documented shape; a real key has not been tried here.
    async fn provider(status: StatusCode) -> (Endpoints, Log) {
        let log = Log::default();
        let (stt, tts) = (log.clone(), log.clone());
        let app = Router::new().fallback(post(move |uri: Uri, headers: HeaderMap, body: Bytes| {
            let (stt, tts) = (stt.clone(), tts.clone());
            async move {
                if uri.path().ends_with("transcriptions") || uri.path().ends_with("speech-to-text")
                {
                    record(&stt, &uri, &headers, &body);
                    return (status, Json(serde_json::json!({"text": " hello there "})))
                        .into_response();
                }
                record(&tts, &uri, &headers, &body);
                // Three bytes: the odd one must wait for the rest of its sample.
                (status, vec![1u8, 0, 2]).into_response()
            }
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (
            Endpoints {
                openai: origin.clone(),
                elevenlabs: origin,
            },
            log,
        )
    }

    use axum::response::IntoResponse;

    fn contains(haystack: &[u8], needle: &str) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    }

    async fn transcribes(provider_kind: Provider, path: &str, auth: &str, model: &str) {
        let (endpoints, log) = provider(StatusCode::OK).await;
        let mut stt = endpoints.listener(provider_kind, "k".to_owned());
        let text = tokio::task::spawn_blocking(move || {
            assert_eq!(stt.accept(&[0.5; 160]).unwrap(), None);
            stt.finish().unwrap()
        })
        .await
        .unwrap();
        assert_eq!(text, "hello there");
        let seen = log.lock().unwrap();
        assert_eq!(seen.path, path);
        assert_eq!(seen.auth, auth);
        assert!(contains(&seen.body, model) && contains(&seen.body, "RIFF"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn openai_transcribes_with_its_documented_request() {
        transcribes(
            Provider::Openai,
            "/v1/audio/transcriptions",
            "Bearer k",
            OPENAI_STT,
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn elevenlabs_transcribes_with_its_documented_request() {
        transcribes(
            Provider::Elevenlabs,
            "/v1/speech-to-text",
            "k",
            ELEVENLABS_STT,
        )
        .await;
    }

    async fn speaks(
        kind: Provider,
        voice: Voice,
        path: &str,
        auth: &str,
        parts: &[&str],
        rate: u32,
    ) {
        let (endpoints, log) = provider(StatusCode::OK).await;
        let tts = endpoints.speaker(kind, "k".to_owned(), voice);
        let heard = tokio::task::spawn_blocking(move || {
            let mut heard = Vec::new();
            tts.speak("Hi.", &mut |audio| {
                heard.push((audio.rate, audio.pcm));
                true
            })
            .unwrap();
            heard
        })
        .await
        .unwrap();
        assert_eq!(heard, [(rate, vec![1, 0])]);
        let seen = log.lock().unwrap();
        assert_eq!(seen.path, path);
        assert_eq!(seen.auth, auth);
        for part in parts {
            assert!(contains(&seen.body, part), "{part} in the request");
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn openai_speaks_with_its_documented_request() {
        speaks(
            Provider::Openai,
            Voice::Male,
            "/v1/audio/speech",
            "Bearer k",
            &[OPENAI_TTS, "onyx", "\"pcm\""],
            OPENAI_RATE,
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn elevenlabs_speaks_with_its_documented_request() {
        speaks(
            Provider::Elevenlabs,
            Voice::Female,
            "/v1/text-to-speech/21m00Tcm4TlvDq8ikWAM/stream?output_format=pcm_16000",
            "k",
            &[ELEVENLABS_TTS],
            SAMPLE_RATE,
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refusal_reports_the_status() {
        let (endpoints, _) = provider(StatusCode::UNAUTHORIZED).await;
        let tts = endpoints.speaker(Provider::Openai, "k".to_owned(), Voice::Male);
        let refused = tokio::task::spawn_blocking(move || tts.speak("Hi.", &mut |_| true))
            .await
            .unwrap();
        assert!(matches!(refused, Err(SpeechError::Refused(401))));
    }
}
