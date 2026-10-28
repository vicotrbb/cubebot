//! Server-sent events carrying one JSON payload each.
use crate::{Error, unavailable};
use reqwest::Response;
use serde_json::Value;

/// Bounds one event, not a response: OpenAI repeats the whole output in its final event.
const EVENT_BYTES: usize = 16 * 1024 * 1024;

pub(crate) struct Reader {
    response: Response,
    buffer: Vec<u8>,
}
impl Reader {
    pub fn new(response: Response) -> Self {
        Self {
            response,
            buffer: Vec::new(),
        }
    }

    /// The next event's data, or `None` when the body ends. Event names, comments and
    /// keep-alives are skipped; both providers repeat the event type in the data.
    pub async fn next(&mut self) -> Result<Option<Value>, Error> {
        loop {
            if let Some(end) = self.buffer.windows(2).position(|pair| pair == b"\n\n") {
                let event: Vec<u8> = self.buffer.drain(..end + 2).collect();
                if let Some(data) = data(&event)? {
                    return Ok(Some(data));
                }
                continue;
            }
            if self.buffer.len() > EVENT_BYTES {
                return Err(Error::InvalidResponse);
            }
            match self.response.chunk().await.map_err(unavailable)? {
                // JSON never contains a raw carriage return, so CRLF framing reduces to LF.
                Some(chunk) => self
                    .buffer
                    .extend(chunk.iter().filter(|byte| **byte != b'\r')),
                None => return Ok(None),
            }
        }
    }
}

fn data(event: &[u8]) -> Result<Option<Value>, Error> {
    let text = std::str::from_utf8(event).map_err(|_| Error::InvalidResponse)?;
    let data: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|line| line.strip_prefix(' ').unwrap_or(line))
        .collect();
    if data.is_empty() {
        return Ok(None);
    }
    serde_json::from_str(&data.join("\n"))
        .map(Some)
        .map_err(|_| Error::InvalidResponse)
}

#[cfg(test)]
mod tests {
    use super::data;

    #[test]
    fn events_join_data_lines_and_skip_comments() {
        assert_eq!(
            data(b"event: x\ndata: {\"a\":\ndata: 1}\n\n").unwrap(),
            Some(serde_json::json!({"a":1}))
        );
        assert_eq!(data(b": keep-alive\n\n").unwrap(), None);
        assert!(data(b"data: not json\n\n").is_err());
    }
}
