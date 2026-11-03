//! The arbiter's [`Screen`] and [`Input`] on the outer X display (docs/adr/sandbox.md#desktop-as-a-vmm-device), which only root can
//! reach: frames are GetImage of its root window, and input is XTEST on it, which Xephyr's window
//! receives as device events and passes to the tool user's display nested inside it.
use super::{Event, HEIGHT, Input, Screen, WIDTH};
use std::{io, os::unix::net::UnixStream};
use x11rb::{
    connection::Connection,
    protocol::{
        xproto::{ConnectionExt as _, ImageFormat, InputFocus, Window},
        xtest::ConnectionExt as _,
    },
    rust_connection::{DefaultStream, RustConnection},
    wrapper::ConnectionExt as _,
};

const KEY_PRESS: u8 = 2;
const BUTTON_PRESS: u8 = 4;
const MOTION: u8 = 6;
/// evdev's BTN_LEFT, BTN_RIGHT and BTN_MIDDLE as X buttons.
const BUTTONS: [(u16, u8); 3] = [(0x110, 1), (0x111, 3), (0x112, 2)];

pub struct Display {
    connection: RustConnection,
    root: Window,
    /// The pointer as the last absolute events left it.
    at: [u32; 2],
}

/// Connects to display `number`'s socket with an MIT-MAGIC-COOKIE-1 `cookie`.
pub fn connect(number: u32, cookie: &[u8]) -> io::Result<Display> {
    let stream = UnixStream::connect(format!("/tmp/.X11-unix/X{number}"))?;
    let (stream, _) = DefaultStream::from_unix_stream(stream)?;
    let connection = RustConnection::connect_to_stream_with_auth_info(
        stream,
        0,
        b"MIT-MAGIC-COOKIE-1".to_vec(),
        cookie.to_vec(),
    )
    .map_err(io::Error::other)?;
    let root = connection.setup().roots[0].root;
    Ok(Display {
        connection,
        root,
        at: [0, 0],
    })
}

impl Display {
    /// Gives keyboard focus to the display's only top-level window, Xephyr's, since no window
    /// manager runs on it to follow the pointer.
    pub fn focus(&self) -> io::Result<()> {
        let tree = self
            .connection
            .query_tree(self.root)
            .map_err(io::Error::other)?
            .reply()
            .map_err(io::Error::other)?;
        let [window] = tree.children[..] else {
            return Err(io::Error::other(format!(
                "{} top-level windows on the outer display, not one",
                tree.children.len()
            )));
        };
        self.connection
            .set_input_focus(InputFocus::PARENT, window, x11rb::CURRENT_TIME)
            .map_err(io::Error::other)?;
        self.connection.sync().map_err(io::Error::other)
    }
}

impl Screen for Display {
    fn capture(&mut self) -> io::Result<Vec<u8>> {
        let image = self
            .connection
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.root,
                0,
                0,
                WIDTH as u16,
                HEIGHT as u16,
                !0,
            )
            .map_err(io::Error::other)?
            .reply()
            .map_err(io::Error::other)?;
        if image.data.len() != WIDTH as usize * HEIGHT as usize * 4 {
            return Err(io::Error::other("the outer display is not 32-bit BGRX"));
        }
        Ok(image.data)
    }
}

impl Input for Display {
    /// Sends the events as XTEST requests, then waits for the server to have taken them.
    fn send(&mut self, events: &[Event]) -> io::Result<()> {
        for (kind, detail, x, y) in fake(events, &mut self.at)? {
            self.connection
                .xtest_fake_input(kind, detail, x11rb::CURRENT_TIME, self.root, x, y, 0)
                .map_err(io::Error::other)?;
        }
        self.connection.sync().map_err(io::Error::other)?;
        // A request the server refused shows up as an error event after the round trip.
        match self.connection.poll_for_event() {
            Ok(Some(x11rb::protocol::Event::Error(error))) => {
                Err(io::Error::other(format!("{:?}", error.error_kind)))
            }
            Ok(_) => Ok(()),
            Err(error) => Err(io::Error::other(error)),
        }
    }
}

/// XTEST fake inputs (type, detail, x, y) for evdev events: key codes are evdev codes plus 8, as
/// XKB's evdev rules number them, absolute moves become one motion at each SYN, and each wheel
/// click a press and release of buttons 4 to 7.
fn fake(events: &[Event], at: &mut [u32; 2]) -> io::Result<Vec<(u8, u8, i16, i16)>> {
    let mut fakes = Vec::new();
    let mut moved = false;
    for &(kind, code, value) in events {
        match (kind, code) {
            (0, _) if std::mem::take(&mut moved) => {
                fakes.push((MOTION, 0, at[0] as i16, at[1] as i16));
            }
            (0, _) => (),
            (1, _) => fakes.push(pressed(code, value)?),
            (3, axis @ (0 | 1)) => (at[usize::from(axis)], moved) = (value, true),
            (2, wheel @ (6 | 8)) => {
                let button = wheel_button(wheel, value);
                fakes.push((BUTTON_PRESS, button, 0, 0));
                fakes.push((BUTTON_PRESS + 1, button, 0, 0));
            }
            _ => return Err(io::Error::other(format!("unsupported event {kind}:{code}"))),
        }
    }
    Ok(fakes)
}

/// A key or button press, or its release when `value` is 0.
fn pressed(code: u16, value: u32) -> io::Result<(u8, u8, i16, i16)> {
    let released = u8::from(value == 0);
    Ok(match BUTTONS.iter().find(|(evdev, _)| *evdev == code) {
        Some(&(_, button)) => (BUTTON_PRESS + released, button, 0, 0),
        None => (KEY_PRESS + released, key(code)?, 0, 0),
    })
}

/// REL_WHEEL counts up and REL_HWHEEL right; X has up 4, down 5, left 6 and right 7.
fn wheel_button(wheel: u16, value: u32) -> u8 {
    match (wheel, value == 1) {
        (8, true) => 4,
        (8, false) => 5,
        (_, false) => 6,
        (_, true) => 7,
    }
}

fn key(code: u16) -> io::Result<u8> {
    u8::try_from(code + 8).map_err(|_| io::Error::other(format!("no X key code for {code}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evdev_events_become_xtest_inputs() {
        let mut at = [0, 0];
        let click = [
            (3, 0, 10),
            (3, 1, 20),
            (0, 0, 0),
            (1, 0x111, 1),
            (0, 0, 0),
            (1, 0x111, 0),
            (0, 0, 0),
        ];
        assert_eq!(
            fake(&click, &mut at).unwrap(),
            [(6, 0, 10, 20), (4, 3, 0, 0), (5, 3, 0, 0)]
        );
        assert_eq!(at, [10, 20]);
        let shifted_a = [(1, 42, 1), (0, 0, 0), (1, 30, 1), (1, 30, 0), (1, 42, 0)];
        assert_eq!(
            fake(&shifted_a, &mut at).unwrap(),
            [(2, 50, 0, 0), (2, 38, 0, 0), (3, 38, 0, 0), (3, 50, 0, 0)]
        );
        let wheel = [(2, 8, u32::MAX), (0, 0, 0), (2, 6, 1), (0, 0, 0)];
        assert_eq!(
            fake(&wheel, &mut at).unwrap(),
            [(4, 5, 0, 0), (5, 5, 0, 0), (4, 7, 0, 0), (5, 7, 0, 0)]
        );
        assert!(fake(&[(1, 300, 1)], &mut at).is_err());
        assert!(fake(&[(4, 0, 0)], &mut at).is_err());
    }
}
