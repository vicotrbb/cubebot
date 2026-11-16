use super::{Audio, Provider, SAMPLE_RATE, SpeechError, Stt, Tts, Voice};
use futures_util::StreamExt;
use serde_json::Value;
use tokio::{runtime::Handle, sync::OnceCell};

const OPENAI_STT: &str = "gpt-4o-mini-transcribe";
const OPENAI_TTS: &str = "gpt-4o-mini-tts";
const OPENAI_RATE: u32 = 24_000;
const ELEVENLABS_STT: &str = "scribe_v1";
const ELEVENLABS_TTS: &str = "eleven_flash_v2_5";
const ELEVENLABS_FEMALE: &str = "EXAVITQu4vr4xnSDxMaL";
const ELEVENLABS_MALE: &str = "JBFqnCBsd6RMkjVDRZzb";
const REASON_LIMIT: usize = 240;

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
            voice_id: OnceCell::new(),
        }
    }

    fn origin(&self, provider: Provider) -> String {
        match provider {
            Provider::Elevenlabs => self.elevenlabs.clone(),
            _ => self.openai.clone(),
        }
    }
}

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
            return Err(refused(response, &self.key).await);
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
    voice_id: OnceCell<String>,
}

async fn refused(response: reqwest::Response, key: &str) -> SpeechError {
    let status = response.status().as_u16();
    let reason = response
        .json::<Value>()
        .await
        .ok()
        .and_then(|body| explained(&body, key));
    SpeechError::Refused(status, reason)
}

fn explained(body: &Value, key: &str) -> Option<String> {
    let message = ["/detail/message", "/error/message", "/detail", "/message"]
        .iter()
        .find_map(|pointer| body.pointer(pointer).and_then(Value::as_str))?;
    let clean = if key.is_empty() {
        message.to_owned()
    } else {
        message.replace(key, "[key]")
    };
    let clean: String = clean.trim().chars().take(REASON_LIMIT).collect();
    (!clean.is_empty()).then_some(clean)
}

fn usual(voice: Voice) -> &'static str {
    match voice {
        Voice::Female => ELEVENLABS_FEMALE,
        Voice::Male => ELEVENLABS_MALE,
    }
}

fn id(entry: &Value) -> Option<&str> {
    entry.get("voice_id").and_then(Value::as_str)
}

fn chosen(listed: &Value, voice: Voice) -> Option<String> {
    let voices = listed.get("voices")?.as_array()?;
    let premade = |entry: &&Value| entry.get("category").and_then(Value::as_str) == Some("premade");
    let gender = match voice {
        Voice::Female => "female",
        Voice::Male => "male",
    };
    let gendered =
        |entry: &&Value| entry.pointer("/labels/gender").and_then(Value::as_str) == Some(gender);
    voices
        .iter()
        .find(|entry| id(entry) == Some(usual(voice)))
        .or_else(|| voices.iter().filter(premade).find(gendered))
        .or_else(|| voices.iter().find(premade))
        .or_else(|| voices.first())
        .and_then(id)
        .map(str::to_owned)
}

impl Speaker {
    async fn voice_id(&self) -> &str {
        self.voice_id
            .get_or_init(|| async {
                match self.listed().await {
                    Ok(Some(id)) => id,
                    Ok(None) => usual(self.voice).to_owned(),
                    Err(error) => {
                        tracing::warn!(%error, "elevenlabs voices could not be listed; using the usual voice");
                        usual(self.voice).to_owned()
                    }
                }
            })
            .await
    }

    async fn listed(&self) -> Result<Option<String>, SpeechError> {
        let response = self
            .client
            .get(format!("{}/v1/voices", self.origin))
            .header("xi-api-key", &self.key)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(refused(response, &self.key).await);
        }
        Ok(chosen(&response.json::<Value>().await?, self.voice))
    }

    fn request(&self, text: &str, voice: &str) -> (reqwest::RequestBuilder, u32) {
        match self.provider {
            Provider::Elevenlabs => {
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

    async fn open(&self, text: &str) -> Result<(reqwest::Response, u32), SpeechError> {
        let voice = match self.provider {
            Provider::Elevenlabs => self.voice_id().await,
            _ => "",
        };
        let (request, rate) = self.request(text, voice);
        let response = request.send().await?;
        if !response.status().is_success() {
            return Err(refused(response, &self.key).await);
        }
        Ok((response, rate))
    }
}

impl Tts for Speaker {
    fn speak(&self, text: &str, sink: &mut dyn FnMut(Audio) -> bool) -> Result<(), SpeechError> {
        let (response, rate) = self.runtime.block_on(self.open(text))?;
        let mut body = response.bytes_stream();
        let mut pcm = Vec::new();
        while let Some(chunk) = self.runtime.block_on(body.next()) {
            pcm.extend_from_slice(&chunk?);
            let rest = pcm.split_off(pcm.len() & !1);
            if !pcm.is_empty() && !sink(Audio { rate, pcm }) {
                return Ok(());
            }
            pcm = rest;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        body::Bytes,
        http::{HeaderMap, StatusCode, Uri},
        routing::{get, post},
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
        let voices = serde_json::json!({"voices": [
            {"voice_id": "cloned", "category": "cloned", "labels": {"gender": "female"}},
            {"voice_id": "premade-male", "category": "premade", "labels": {"gender": "male"}},
            {"voice_id": "premade-female", "category": "premade", "labels": {"gender": "female"}},
        ]});
        let app = Router::new()
            .route("/v1/voices", get(move || async move { Json(voices) }))
            .fallback(post(move |uri: Uri, headers: HeaderMap, body: Bytes| {
                let (stt, tts) = (stt.clone(), tts.clone());
                async move {
                    if uri.path().ends_with("transcriptions")
                        || uri.path().ends_with("speech-to-text")
                    {
                        record(&stt, &uri, &headers, &body);
                        return (status, Json(serde_json::json!({"text": " hello there "})))
                            .into_response();
                    }
                    record(&tts, &uri, &headers, &body);
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
        let (sounds, mut heard) = tokio::sync::mpsc::channel(4);
        std::thread::spawn(move || {
            tts.speak("Hi.", &mut |audio| {
                sounds.blocking_send((audio.rate, audio.pcm)).is_ok()
            })
            .unwrap();
        });
        assert_eq!(heard.recv().await, Some((rate, vec![1, 0])));
        assert_eq!(heard.recv().await, None);
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
            "/v1/text-to-speech/premade-female/stream?output_format=pcm_16000",
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
        assert!(matches!(refused, Err(SpeechError::Refused(401, _))));
    }

    #[test]
    fn a_refusal_explains_itself_without_the_key() {
        let body = serde_json::json!({"detail": {"status": "payment_required", "message": "Key sk_secret has no credits left."}});
        assert_eq!(
            explained(&body, "sk_secret").as_deref(),
            Some("Key [key] has no credits left.")
        );
        let openai = serde_json::json!({"error": {"message": "Quota exceeded."}});
        assert_eq!(explained(&openai, "k").as_deref(), Some("Quota exceeded."));
        assert_eq!(
            explained(&serde_json::json!({"detail": {"status": "x"}}), "k"),
            None
        );
        let long = serde_json::json!({"detail": "x".repeat(1000)});
        assert_eq!(
            explained(&long, "k").map(|text| text.len()),
            Some(REASON_LIMIT)
        );
    }

    #[test]
    fn the_voice_is_one_the_account_has() {
        let listed = |voices: serde_json::Value| serde_json::json!({ "voices": voices });
        let usual = listed(serde_json::json!([
            {"voice_id": "premade-female", "category": "premade", "labels": {"gender": "female"}},
            {"voice_id": ELEVENLABS_FEMALE, "category": "premade", "labels": {"gender": "female"}},
        ]));
        assert_eq!(
            chosen(&usual, Voice::Female).as_deref(),
            Some(ELEVENLABS_FEMALE)
        );
        let newer = listed(serde_json::json!([
            {"voice_id": "library", "category": "professional", "labels": {"gender": "male"}},
            {"voice_id": "premade-female", "category": "premade", "labels": {"gender": "female"}},
            {"voice_id": "premade-male", "category": "premade", "labels": {"gender": "male"}},
        ]));
        assert_eq!(chosen(&newer, Voice::Male).as_deref(), Some("premade-male"));
        let unlabelled = listed(serde_json::json!([
            {"voice_id": "library", "category": "professional"},
            {"voice_id": "premade", "category": "premade"},
        ]));
        assert_eq!(
            chosen(&unlabelled, Voice::Female).as_deref(),
            Some("premade")
        );
        assert_eq!(chosen(&listed(serde_json::json!([])), Voice::Male), None);
    }
}
