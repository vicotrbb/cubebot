use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const LOCAL_PORT: u16 = 7400;
pub const VIEWER_ID: &str = "x-cubebot-viewer-id";
pub const VIEWER_NAME: &str = "x-cubebot-viewer-name";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame<'a> {
    Open(u32),
    Data(u32, &'a [u8]),
    Close(u32),
}

pub fn encode(frame: &Frame) -> Vec<u8> {
    let (stream, kind, payload): (u32, u8, &[u8]) = match frame {
        Frame::Open(stream) => (*stream, 0, &[]),
        Frame::Data(stream, data) => (*stream, 1, data),
        Frame::Close(stream) => (*stream, 2, &[]),
    };
    let mut bytes = Vec::with_capacity(5 + payload.len());
    bytes.extend_from_slice(&stream.to_be_bytes());
    bytes.push(kind);
    bytes.extend_from_slice(payload);
    bytes
}

pub fn decode(bytes: &[u8]) -> Option<Frame<'_>> {
    let (head, payload) = bytes.split_at_checked(5)?;
    let stream = u32::from_be_bytes(head[..4].try_into().ok()?);
    match head[4] {
        0 => Some(Frame::Open(stream)),
        1 => Some(Frame::Data(stream, payload)),
        2 => Some(Frame::Close(stream)),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Desired {
    Running,
    Stopped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Reported {
    Starting,
    Running,
    Stopped,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct AppSpec {
    pub slug: String,
    pub name: String,
    pub folder: String,
    pub command: Option<String>,
    pub port: Option<u16>,
    pub tools: Vec<String>,
    pub desired: Desired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppReport {
    State {
        slug: String,
        state: Reported,
        memory: Option<u64>,
    },
    Called {
        slug: String,
        tool: String,
        viewer: Option<Uuid>,
        ok: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AppNotice {
    Changed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_round_trip_and_reject_garbage() {
        for frame in [Frame::Open(1), Frame::Data(2, b"hi"), Frame::Close(3)] {
            assert_eq!(decode(&encode(&frame)), Some(frame));
        }
        assert_eq!(decode(&[0, 0, 0, 1]), None);
        assert_eq!(decode(&[0, 0, 0, 1, 9]), None);
    }

    #[test]
    fn report_and_spec_wire_shapes() {
        let report = AppReport::State {
            slug: "a".into(),
            state: Reported::Running,
            memory: None,
        };
        let text = serde_json::to_string(&report).unwrap();
        assert_eq!(
            text,
            r#"{"type":"state","slug":"a","state":"running","memory":null}"#
        );
        assert_eq!(serde_json::from_str::<AppReport>(&text).unwrap(), report);
        assert!(serde_json::from_str::<AppSpec>(r#"{"extra":1}"#).is_err());
    }
}
