use super::pipeline::{self, Heard, Listener, Sentences};
use super::settings::route;
use super::{Audio, Engines, Speech, SpeechError};
use crate::{
    agents::AgentId,
    identity::{Actor, Identity, IdentityError},
    work::{Reply, ReplyEvent, Work},
};
use axum::extract::ws::{CloseFrame, Message, WebSocket};
use serde_json::{Value, json};
use std::collections::VecDeque;
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;
use tokio::sync::mpsc;
use uuid::Uuid;

const FOLLOW: Duration = Duration::from_millis(100);
const RECHECK: Duration = Duration::from_secs(30);
const AUDIO_BACKLOG: usize = 128;

pub struct Session {
    pub speech: Speech,
    pub work: Work,
    pub identity: Identity,
    pub pool: sqlx::PgPool,
    pub agent: AgentId,
    pub actor: Actor,
    pub token: String,
}

struct Awaited {
    turn: Uuid,
    after: i64,
    muted: bool,
    sentences: Sentences,
    spoke: bool,
}

struct Line {
    call: Uuid,
    audio: mpsc::Sender<Vec<f32>>,
    heard: mpsc::UnboundedReceiver<Heard>,
    speech: mpsc::Receiver<Audio>,
    sentences: mpsc::UnboundedSender<(u64, String)>,
    epoch: Arc<AtomicU64>,
    awaited: VecDeque<Awaited>,
}

fn text(value: &Value) -> Message {
    Message::Text(value.to_string().into())
}

fn samples(bytes: &[u8]) -> Option<Vec<f32>> {
    bytes.len().is_multiple_of(2).then(|| {
        bytes
            .chunks_exact(2)
            .map(|pair| f32::from(i16::from_le_bytes([pair[0], pair[1]])) / 32768.0)
            .collect()
    })
}

fn frame(audio: &Audio) -> Message {
    let mut frame = Vec::with_capacity(4 + audio.pcm.len());
    frame.extend_from_slice(&audio.rate.to_le_bytes());
    frame.extend_from_slice(&audio.pcm);
    Message::Binary(frame.into())
}

impl Session {
    #[tracing::instrument(skip_all, fields(agent = %self.agent.0, actor = %self.actor.id))]
    pub async fn run(self, mut socket: WebSocket) {
        let reason = match self.call(&mut socket).await {
            Ok(()) => return,
            Err(reason) => reason,
        };
        tracing::warn!(reason = %reason, "call ended");
        let close = CloseFrame {
            code: 1011,
            reason: reason.chars().take(120).collect::<String>().into(),
        };
        let _ = socket.send(Message::Close(Some(close))).await;
    }

    async fn call(&self, socket: &mut WebSocket) -> Result<(), String> {
        let route = route(&self.pool, self.agent).await.map_err(|error| {
            tracing::warn!(?error, "speech settings unreadable");
            "the speech settings could not be read".to_owned()
        })?;
        let Some(engines) = self.prepare(socket, route).await? else {
            return Ok(());
        };
        let call = self
            .work
            .start_call(&self.actor, self.agent, route)
            .await
            .map_err(|error| {
                tracing::warn!(?error, "call not recorded");
                "the call could not be recorded".to_owned()
            })?;
        let ready =
            json!({"kind":"ready","call":call,"stt":route.stt,"tts":route.tts,"voice":route.voice});
        let mut line = start(call, engines);
        let served = match socket.send(text(&ready)).await {
            Ok(()) => self.serve(socket, &mut line).await,
            Err(_) => Ok(()),
        };
        if let Err(error) = self.work.end_call(call).await {
            tracing::warn!(?error, %call, "call end not recorded");
        }
        served
    }

    async fn prepare(
        &self,
        socket: &mut WebSocket,
        route: super::Route,
    ) -> Result<Option<Engines>, String> {
        let (progress, mut updates) = mpsc::unbounded_channel();
        let speech = self.speech.clone();
        let agent = self.agent;
        let mut loading = tokio::spawn(async move {
            let report = move |model: &'static str, received: u64, total: u64| {
                let _ = progress.send(
                    json!({"kind":"preparing","model":model,"received":received,"total":total}),
                );
            };
            speech.engines(agent, route, &report).await
        });
        loop {
            tokio::select! {
                engines = &mut loading => {
                    let engines = engines.map_err(|error| error.to_string())?;
                    return engines.map(Some).map_err(|error: SpeechError| {
                        tracing::warn!(?error, "speech engines unavailable");
                        error.to_string()
                    });
                }
                Some(update) = updates.recv() => {
                    socket.send(text(&update)).await.map_err(|error| error.to_string())?;
                }
                message = socket.recv() => {
                    if matches!(message, Some(Ok(Message::Close(_)) | Err(_)) | None) {
                        return Ok(None);
                    }
                }
            }
        }
    }

    async fn serve(&self, socket: &mut WebSocket, line: &mut Line) -> Result<(), String> {
        let mut follow = tokio::time::interval(FOLLOW);
        follow.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut recheck = tokio::time::interval(RECHECK);
        recheck.reset();
        loop {
            let waiting = !line.awaited.is_empty();
            tokio::select! {
                message = socket.recv() => if !line.received(message) {
                    return Ok(());
                },
                Some(heard) = line.heard.recv() => {
                    let out = self.heard(line, heard).await?;
                    send_all(socket, out).await?;
                }
                Some(audio) = line.speech.recv() => {
                    socket.send(frame(&audio)).await.map_err(|error| error.to_string())?;
                }
                _ = follow.tick(), if waiting => {
                    let out = self.follow(line).await?;
                    send_all(socket, out).await?;
                }
                _ = recheck.tick() => self.recheck().await?,
            }
        }
    }

    async fn recheck(&self) -> Result<(), String> {
        let ended = || "access to this agent ended".to_owned();
        let actor = self
            .identity
            .authenticate(&self.token)
            .await
            .map_err(|_| ended())?;
        self.work
            .may_call(&actor, self.agent)
            .await
            .map_err(|_| ended())
    }

    async fn heard(&self, line: &mut Line, heard: Heard) -> Result<Vec<Value>, String> {
        Ok(match heard {
            Heard::Speech => {
                line.interrupt();
                vec![json!({"kind":"interrupt"})]
            }
            Heard::Partial(text) => vec![json!({"kind":"partial","text":text})],
            Heard::Failed(error) => {
                tracing::warn!(?error, "speech failed");
                vec![json!({"kind":"notice","text":error.to_string()})]
            }
            Heard::Utterance(text) => vec![self.say(line, text).await?],
        })
    }

    async fn say(&self, line: &mut Line, text: String) -> Result<Value, String> {
        let said = self
            .work
            .say_in_call(&self.actor, self.agent, line.call, &text)
            .await;
        let turn = said.map_err(|error| match error {
            IdentityError::NotFound | IdentityError::Forbidden | IdentityError::Unauthorized => {
                "access to this agent ended".to_owned()
            }
            error => {
                tracing::warn!(?error, "call turn not recorded");
                "your words could not be recorded".to_owned()
            }
        })?;
        line.awaited.push_back(Awaited {
            turn,
            after: 0,
            muted: false,
            sentences: Sentences::default(),
            spoke: false,
        });
        Ok(json!({"kind":"user","turn":turn,"text":text}))
    }

    async fn follow(&self, line: &mut Line) -> Result<Vec<Value>, String> {
        let Some(awaited) = line.awaited.front_mut() else {
            return Ok(Vec::new());
        };
        let reply = self
            .work
            .reply(awaited.turn, &mut awaited.after)
            .await
            .map_err(|error| {
                tracing::warn!(?error, "reply unreadable");
                "the reply could not be read".to_owned()
            })?;
        let out = line.relay(reply);
        Ok(out)
    }
}

async fn send_all(socket: &mut WebSocket, values: Vec<Value>) -> Result<(), String> {
    for value in values {
        socket
            .send(text(&value))
            .await
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn start(call: Uuid, engines: Engines) -> Line {
    let Engines { vad, stt, tts } = engines;
    let (audio, audio_rx) = mpsc::channel(AUDIO_BACKLOG);
    let (heard_tx, heard) = mpsc::unbounded_channel();
    let (sentences, sentences_rx) = mpsc::unbounded_channel();
    let (speech_tx, speech) = mpsc::channel(32);
    let epoch = Arc::new(AtomicU64::new(0));
    let listener = Listener::new(vad, stt);
    let failures = heard_tx.clone();
    std::thread::spawn(move || pipeline::listen(listener, audio_rx, heard_tx));
    let speaking = epoch.clone();
    std::thread::spawn(move || pipeline::speak(tts, sentences_rx, speaking, speech_tx, failures));
    Line {
        call,
        audio,
        heard,
        speech,
        sentences,
        epoch,
        awaited: VecDeque::new(),
    }
}

impl Line {
    fn received(&self, message: Option<Result<Message, axum::Error>>) -> bool {
        match message {
            Some(Ok(Message::Binary(bytes))) => self.hear(&bytes),
            Some(Ok(Message::Close(_)) | Err(_)) | None => return false,
            Some(Ok(_)) => {}
        }
        true
    }

    fn hear(&self, bytes: &[u8]) {
        let taken = samples(bytes).is_some_and(|samples| self.audio.try_send(samples).is_ok());
        if !taken {
            tracing::warn!(
                bytes = bytes.len(),
                "audio frame dropped: split sample or listener behind"
            );
        }
    }

    fn interrupt(&mut self) {
        self.epoch.fetch_add(1, Ordering::SeqCst);
        for awaited in &mut self.awaited {
            awaited.muted = true;
        }
    }

    fn say(&self, sentence: String) {
        let _ = self
            .sentences
            .send((self.epoch.load(Ordering::SeqCst), sentence));
    }

    fn relay(&mut self, reply: Reply) -> Vec<Value> {
        let Some(mut awaited) = self.awaited.pop_front() else {
            return Vec::new();
        };
        let mut out: Vec<Value> = reply
            .events
            .into_iter()
            .map(|event| self.show(&mut awaited, event))
            .collect();
        self.settle(awaited, reply.status, reply.response, reply.error, &mut out);
        out
    }

    fn show(&self, awaited: &mut Awaited, event: ReplyEvent) -> Value {
        match event {
            ReplyEvent::Tool(name) => json!({"kind":"tool","turn":awaited.turn,"name":name}),
            ReplyEvent::Text(text) => {
                self.speak_part(awaited, &text);
                json!({"kind":"agent","turn":awaited.turn,"text":text})
            }
        }
    }

    fn settle(
        &mut self,
        mut awaited: Awaited,
        status: String,
        response: Option<String>,
        error: Option<String>,
        out: &mut Vec<Value>,
    ) {
        let turn = awaited.turn;
        match status.as_str() {
            "complete" => {
                self.finish_speaking(&mut awaited, response.as_deref());
                out.push(json!({"kind":"answered","turn":turn,"text":response}));
            }
            "failed" | "cancelled" | "waiting" => {
                self.finish_speaking(&mut awaited, None);
                out.push(json!({"kind":"unanswered","turn":turn,"status":status,"error":error}));
            }
            status => {
                if out.is_empty() && status == "queued" {
                    out.push(json!({"kind":"queued","turn":turn}));
                }
                self.awaited.push_front(awaited);
            }
        }
    }

    fn speak_part(&self, awaited: &mut Awaited, text: &str) {
        for sentence in awaited.sentences.push(text) {
            if !awaited.muted {
                awaited.spoke = true;
                self.say(sentence);
            }
        }
    }

    fn finish_speaking(&self, awaited: &mut Awaited, whole: Option<&str>) {
        let mut rest = awaited.sentences.finish();
        if !awaited.spoke
            && rest.is_empty()
            && let Some(whole) = whole
        {
            rest = awaited.sentences.push(whole);
            rest.extend(awaited.sentences.finish());
        }
        if !awaited.muted {
            for sentence in rest {
                self.say(sentence);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_frames_decode_and_reject_a_split_sample() {
        assert_eq!(samples(&[0, 0x40, 0, 0xC0]), Some(vec![0.5, -0.5]));
        assert_eq!(samples(&[0]), None);
    }

    #[test]
    fn an_audio_frame_leads_with_its_rate() {
        let Message::Binary(frame) = frame(&Audio {
            rate: 22050,
            pcm: vec![1, 2],
        }) else {
            panic!("binary");
        };
        assert_eq!(&frame[..], [0x22, 0x56, 0, 0, 1, 2]);
    }
}
