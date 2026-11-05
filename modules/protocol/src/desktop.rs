//! Each request gets one [`Response`] line. The arbiter tags every request with its source from the stream it arrived on, never from its body.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A `kvm` guest's arbiter opens host channel streams to `HOST_CID:HOST_PORT` from a reserved source port.
pub const HOST_PORT: u32 = 1026;
pub const LINE_LIMIT: usize = 64 << 10;
pub const PERSON_HAS_CONTROL: &str = "a person has control";
/// Refused until the agent takes a screenshot: what it last saw may no longer be on the screen.
pub const REOBSERVE: &str = "a person had control; take a screenshot before further input";
/// The refusal of every agent request, screenshots included, while a person completes a login the agent asked for.
pub const LOGIN_PENDING: &str =
    "a person is logging in for you; wait for the message that the login handoff finished";
pub const TILE: u32 = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Agent,
    Watch,
    Human,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// Coordinates are screen pixels from the top left.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Screenshot,
    /// The response carries the mode, which the agent's browser tools ask before input they send to Chromium directly.
    Mode,
    Move {
        x: u32,
        y: u32,
    },
    Click {
        #[serde(default)]
        at: Option<[u32; 2]>,
        button: Button,
        count: u8,
        #[serde(default)]
        hold: Vec<String>,
    },
    Drag {
        path: Vec<[u32; 2]>,
        #[serde(default)]
        hold: Vec<String>,
    },
    /// Wheel clicks, positive down and right.
    Scroll {
        #[serde(default)]
        at: Option<[u32; 2]>,
        down: i32,
        right: i32,
        #[serde(default)]
        hold: Vec<String>,
    },
    Type {
        text: String,
    },
    Key {
        keys: Vec<String>,
    },
}

impl Action {
    pub fn is_input(&self) -> bool {
        !matches!(self, Self::Screenshot | Self::Mode)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRequest {
    pub call: Uuid,
    pub action: Action,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HostRequest {
    /// The first frame of a connection has every tile.
    Frame,
    Input {
        account: Uuid,
        action: Action,
    },
    /// While one person holds control, any change must name that person.
    Mode {
        mode: Mode,
        #[serde(default)]
        account: Option<Uuid>,
        /// With `human`: for a pending login handoff, so the agent's screenshots and mode queries are refused too until control changes.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        handoff: bool,
    },
    /// `None` is the agent's `vault_fill`, refused like its own input; only `item` and the text's length are logged.
    Fill {
        item: Uuid,
        text: String,
        #[serde(default)]
        account: Option<Uuid>,
    },
}

/// Printable ASCII and spaces on a US layout. A vault refuses to save what it could not fill.
pub fn typable(text: &str) -> bool {
    text.chars().all(|c| c == ' ' || c.is_ascii_graphic())
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The screen as a base64 PNG.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub png: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiles: Vec<Tile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<Pointer>,
}

/// One [`TILE`]-sized region at (`x`, `y`), smaller at the screen's right and bottom edges.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    pub png: String,
}

/// Where the pointer is and who last moved it: a person's account, or the agent when `None`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pointer {
    pub at: [u32; 2],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<Uuid>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_have_a_stable_wire_form() {
        let request: AgentRequest = serde_json::from_str(
            r#"{"call":"00000000-0000-0000-0000-000000000001","action":{"action":"click","at":[3,4],"button":"left","count":2}}"#,
        )
        .unwrap();
        assert_eq!(
            request.action,
            Action::Click {
                at: Some([3, 4]),
                button: Button::Left,
                count: 2,
                hold: Vec::new()
            }
        );
        let mode: HostRequest = serde_json::from_str(
            r#"{"mode":{"mode":"human","account":"00000000-0000-0000-0000-000000000002"}}"#,
        )
        .unwrap();
        assert!(matches!(
            mode,
            HostRequest::Mode {
                mode: Mode::Human,
                account: Some(_),
                handoff: false,
            }
        ));
        let refused = Response {
            mode: Mode::Human,
            error: Some(PERSON_HAS_CONTROL.into()),
            ..Response::default()
        };
        assert_eq!(
            serde_json::to_string(&refused).unwrap(),
            r#"{"mode":"human","error":"a person has control"}"#
        );
        assert!(serde_json::from_str::<Action>(r#"{"action":"zoom"}"#).is_err());
        assert_eq!(
            serde_json::from_str::<HostRequest>(r#""frame""#).unwrap(),
            HostRequest::Frame
        );
    }
}
