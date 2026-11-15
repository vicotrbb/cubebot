use crate::agents::Avatar;
use axum::body::Bytes;
use axum::extract::ws::Message;
use cubebot_protocol::desktop::Tile;
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use uuid::Uuid;

pub const FRAME_LIMIT: usize = 256 * 1024;
pub const PEOPLE_RATE: u32 = 16_000;
const AUDIO: u8 = 1;
const CAMERA: u8 = 2;
const SCREEN: u8 = 3;
const VIDEO_HEADER: usize = 5;
const SENDER_FRAMES: u32 = 200;
const SENDER_BYTES: usize = 4_000_000 / 8;
const SENDER_WINDOW: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Camera,
    Screen,
}

impl Stream {
    fn byte(self) -> u8 {
        match self {
            Self::Camera => CAMERA,
            Self::Screen => SCREEN,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum Media {
    Audio(Bytes),
    Video {
        stream: Stream,
        key: bool,
        payload: Bytes,
    },
}

pub fn media(frame: &Bytes) -> Option<Media> {
    let kind = *frame.first()?;
    let payload = frame.slice(1..);
    match kind {
        AUDIO if !payload.is_empty() && payload.len().is_multiple_of(2) => {
            Some(Media::Audio(payload))
        }
        CAMERA | SCREEN if payload.len() > VIDEO_HEADER => Some(Media::Video {
            stream: if kind == CAMERA {
                Stream::Camera
            } else {
                Stream::Screen
            },
            key: payload[0] & 1 == 1,
            payload,
        }),
        _ => None,
    }
}

pub fn audio(source: u16, rate: u32, pcm: &[u8]) -> Bytes {
    let mut frame = Vec::with_capacity(7 + pcm.len());
    frame.push(AUDIO);
    frame.extend_from_slice(&source.to_le_bytes());
    frame.extend_from_slice(&rate.to_le_bytes());
    frame.extend_from_slice(pcm);
    frame.into()
}

pub fn video(stream: Stream, source: u16, payload: &[u8]) -> Bytes {
    let mut frame = Vec::with_capacity(3 + payload.len());
    frame.push(stream.byte());
    frame.extend_from_slice(&source.to_le_bytes());
    frame.extend_from_slice(payload);
    frame.into()
}

pub struct Budget {
    window: Instant,
    frames: u32,
    bytes: usize,
}

impl Budget {
    pub fn new(now: Instant) -> Self {
        Self {
            window: now,
            frames: 0,
            bytes: 0,
        }
    }

    pub fn allows(&mut self, size: usize, now: Instant) -> bool {
        if now.duration_since(self.window) >= SENDER_WINDOW {
            *self = Self::new(now);
        }
        if self.frames >= SENDER_FRAMES || self.bytes + size > SENDER_BYTES {
            return false;
        }
        self.frames += 1;
        self.bytes += size;
        true
    }
}

#[derive(Debug, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Client {
    Hello {
        #[serde(default)]
        name: Option<String>,
    },
    State {
        mic: bool,
        camera: bool,
        screen: bool,
        hand: bool,
    },
    Track {
        stream: Stream,
        codec: Option<String>,
        width: u32,
        height: u32,
    },
    WantKeyframe {
        source: u16,
        stream: Stream,
    },
    Admit {
        id: String,
    },
    Deny {
        id: String,
    },
    Remove {
        source: u16,
    },
    Lock {
        locked: bool,
    },
    Present {
        source: u16,
        share: bool,
    },
    End,
    Ping,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Person,
    Guest,
    Agent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentState {
    Listening,
    Thinking,
    Speaking,
}

#[derive(Clone, Debug, Serialize)]
pub struct Participant {
    pub slot: u16,
    pub kind: Kind,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar: Option<Avatar>,
    pub host: bool,
    pub mic: bool,
    pub camera: bool,
    pub screen: bool,
    pub hand: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub can_present: Option<bool>,
}

#[derive(Clone, Debug, Serialize)]
pub struct Waiting {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Server {
    Waiting,
    Welcome {
        you: u16,
        title: String,
        host: bool,
        member: bool,
        open_join: bool,
        locked: bool,
        captions: bool,
    },
    Roster {
        participants: Vec<Participant>,
    },
    Track {
        source: u16,
        stream: Stream,
        codec: Option<String>,
        width: u32,
        height: u32,
    },
    Keyframe {
        stream: Stream,
    },
    Caption {
        source: u16,
        id: String,
        text: String,
        r#final: bool,
    },
    Agent {
        source: u16,
        state: AgentState,
    },
    Interrupt {
        source: u16,
    },
    Desktop {
        source: u16,
        #[serde(skip_serializing_if = "Option::is_none")]
        tiles: Option<Vec<Tile>>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        ended: bool,
    },
    Lobby {
        waiting: Vec<Waiting>,
    },
    Meeting {
        locked: bool,
        open_join: bool,
    },
    Preparing {
        model: String,
        received: u64,
        total: u64,
    },
    Notice {
        text: String,
    },
    Denied,
    Removed,
    Ended,
}

impl Server {
    pub fn message(&self) -> Message {
        Message::Text(serde_json::to_string(self).unwrap_or_default().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn text(server: &Server) -> Value {
        let Message::Text(text) = server.message() else {
            panic!("a text frame");
        };
        serde_json::from_str(&text).unwrap()
    }

    #[test]
    fn browser_frames_parse_by_their_type_byte() {
        let microphone = Bytes::from_static(&[1, 0x10, 0x00, 0xff, 0x7f]);
        assert_eq!(
            media(&microphone),
            Some(Media::Audio(Bytes::from_static(&[0x10, 0x00, 0xff, 0x7f])))
        );
        let camera = Bytes::from_static(&[2, 1, 10, 0, 0, 0, 0xaa]);
        assert_eq!(
            media(&camera),
            Some(Media::Video {
                stream: Stream::Camera,
                key: true,
                payload: Bytes::from_static(&[1, 10, 0, 0, 0, 0xaa]),
            })
        );
        let screen = Bytes::from_static(&[3, 0, 10, 0, 0, 0, 0xaa]);
        assert!(matches!(
            media(&screen),
            Some(Media::Video {
                stream: Stream::Screen,
                key: false,
                ..
            })
        ));
        for refused in [
            &[][..],
            &[1],
            &[1, 0, 0, 0],
            &[2, 1, 0, 0, 0, 0],
            &[4, 0, 0],
        ] {
            assert_eq!(media(&Bytes::copy_from_slice(refused)), None, "{refused:?}");
        }
    }

    #[test]
    fn relayed_frames_carry_the_source_slot_and_audio_its_rate() {
        assert_eq!(
            &audio(258, 16_000, &[9, 8])[..],
            &[1, 2, 1, 0x80, 0x3e, 0, 0, 9, 8]
        );
        assert_eq!(
            &video(Stream::Screen, 7, &[1, 10, 0, 0, 0, 0xaa])[..],
            &[3, 7, 0, 1, 10, 0, 0, 0, 0xaa]
        );
    }

    #[test]
    fn a_sender_is_held_to_200_frames_and_4_megabits_a_second() {
        let start = Instant::now();
        let mut frames = Budget::new(start);
        assert!((0..200).all(|_| frames.allows(10, start)));
        assert!(!frames.allows(10, start));
        assert!(frames.allows(10, start + SENDER_WINDOW));
        let mut bytes = Budget::new(start);
        assert!(bytes.allows(FRAME_LIMIT, start));
        assert!(!bytes.allows(FRAME_LIMIT, start));
        assert!(bytes.allows(SENDER_BYTES - FRAME_LIMIT, start));
        assert!(!bytes.allows(1, start));
    }

    #[test]
    fn browser_messages_are_tagged_by_kind() {
        let parse = |value: Value| serde_json::from_value::<Client>(value).unwrap();
        assert_eq!(
            parse(json!({"kind": "hello"})),
            Client::Hello { name: None }
        );
        assert_eq!(
            parse(json!({"kind": "want_keyframe", "source": 3, "stream": "screen"})),
            Client::WantKeyframe {
                source: 3,
                stream: Stream::Screen
            }
        );
        assert_eq!(
            parse(
                json!({"kind": "track", "stream": "camera", "codec": null, "width": 0, "height": 0})
            ),
            Client::Track {
                stream: Stream::Camera,
                codec: None,
                width: 0,
                height: 0
            }
        );
        assert!(serde_json::from_value::<Client>(json!({"kind": "shout"})).is_err());
    }

    #[test]
    fn server_messages_match_the_console_contract() {
        assert_eq!(text(&Server::Ended), json!({"kind": "ended"}));
        assert_eq!(
            text(&Server::Caption {
                source: 2,
                id: "u1".into(),
                text: "Hi".into(),
                r#final: true
            }),
            json!({"kind": "caption", "source": 2, "id": "u1", "text": "Hi", "final": true})
        );
        assert_eq!(
            text(&Server::Desktop {
                source: 4,
                tiles: None,
                ended: true
            }),
            json!({"kind": "desktop", "source": 4, "ended": true})
        );
        let guest = Participant {
            slot: 1,
            kind: Kind::Guest,
            name: "Dee".into(),
            account: None,
            agent: None,
            avatar: None,
            host: false,
            mic: true,
            camera: false,
            screen: false,
            hand: false,
            can_present: None,
        };
        assert_eq!(
            text(&Server::Roster {
                participants: vec![guest]
            }),
            json!({"kind": "roster", "participants": [{"slot": 1, "kind": "guest", "name": "Dee",
                "host": false, "mic": true, "camera": false, "screen": false, "hand": false}]})
        );
    }
}
