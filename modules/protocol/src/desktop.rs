//! The desktop arbiter's requests (ADR 0085). The guest agent opens streams to `HOST_CID:PORT`
//! and sends one [`AgentRequest`] JSON line per request; the sandbox host channel carries
//! [`HostRequest`] lines from the orchestrator. Each request gets one [`Response`] line. The
//! arbiter tags every request with its source from the stream it arrived on, never from its body.
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const PORT: u32 = 1025;
/// Bounds each request line, such as the text of one `type`.
pub const LINE_LIMIT: usize = 64 << 10;
/// The refusal of agent input while a person holds control.
pub const PERSON_HAS_CONTROL: &str = "a person has control";
/// The refusal of agent input after a person handed control back, until the agent takes a
/// screenshot: what it last saw may no longer be on the screen (ADR 0039).
pub const REOBSERVE: &str = "a person had control; take a screenshot before further input";
/// The refusal of every agent request, screenshots included, while a person holds control to
/// complete a login the agent asked for (ADR 0039).
pub const LOGIN_PENDING: &str =
    "a person is logging in for you; wait for the message that the login handoff finished";
/// Changed screen regions travel as square tiles of this many pixels per side.
pub const TILE: u32 = 64;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// The agent drives and no person is connected.
    #[default]
    Agent,
    /// The agent drives and people see the screen.
    Watch,
    /// One person holds control.
    Human,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Button {
    Left,
    Right,
    Middle,
}

/// Coordinates are screen pixels from the top left. `hold` keys stay pressed for the action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Screenshot,
    /// Changes nothing; the response carries the mode, which the agent's browser tools ask
    /// before input they send to Chromium directly.
    Mode,
    Move {
        x: u32,
        y: u32,
    },
    /// At the pointer when `at` is omitted.
    Click {
        #[serde(default)]
        at: Option<[u32; 2]>,
        button: Button,
        count: u8,
        #[serde(default)]
        hold: Vec<String>,
    },
    /// Presses at the first point, moves through the rest and releases at the last.
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
    /// Literal text on a US keyboard layout.
    Type {
        text: String,
    },
    /// Pressed in order and released in reverse, such as `["ctrl", "l"]`.
    Key {
        keys: Vec<String>,
    },
}

impl Action {
    /// Every action but a screenshot or a mode query sends input.
    pub fn is_input(&self) -> bool {
        !matches!(self, Self::Screenshot | Self::Mode)
    }
}

/// From the guest: input or a screenshot for one of the agent's tool calls.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRequest {
    pub call: Uuid,
    pub action: Action,
}

/// From the orchestrator: a person's input or screenshot, a mode change, or the next frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum HostRequest {
    /// The tiles that changed since this host channel connection's last frame; the first frame
    /// of a connection has every tile.
    Frame,
    Input {
        account: Uuid,
        action: Action,
    },
    /// `human` names the person who takes control; while one person holds it, any change must
    /// name that person.
    Mode {
        mode: Mode,
        #[serde(default)]
        account: Option<Uuid>,
        /// With `human`: the person takes control for a pending login handoff, and the agent's
        /// screenshots and mode queries are refused too until control changes again.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        handoff: bool,
    },
}

/// The mode after the request, and its refusal or screenshot.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub mode: Mode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// The screen as a base64 PNG.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub png: Option<String>,
    /// A frame's changed tiles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiles: Vec<Tile>,
    /// A frame's pointer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pointer: Option<Pointer>,
}

/// One [`TILE`]-sized region at (`x`, `y`), smaller at the screen's right and bottom edges.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tile {
    pub x: u32,
    pub y: u32,
    /// A base64 PNG.
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
