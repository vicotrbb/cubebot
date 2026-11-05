use crate::{Error, unavailable};
use reqwest::Response;
use serde_json::Value;

/// Bounds one event, not a response: OpenAI repeats the whole output in its final event.
const EVENT_BYTES: usize = 16 * 1024 * 1024;
/// What [`Reader::next`] returns for Chat Completions' `data: [DONE]`, which is not JSON.
pub(crate) const DONE: &str = "[DONE]";

pub(crate) struct Reader {
    response: Response,
    buffer: Vec<u8>,
    scanned: usize,
}
impl Reader {
    pub fn new(response: Response) -> Self {
        Self {
            response,
            buffer: Vec::new(),
            scanned: 0,
        }
    }

    pub async fn next(&mut self) -> Result<Option<Value>, Error> {
        loop {
            let from = self.scanned;
            if let Some(end) = self.buffer[from..]
                .windows(2)
                .position(|pair| pair == b"\n\n")
            {
                let event: Vec<u8> = self.buffer.drain(..from + end + 2).collect();
                self.scanned = 0;
                if let Some(data) = data(&event)? {
                    return Ok(Some(data));
                }
                continue;
            }
            // The last byte may be the first half of a boundary.
            self.scanned = self.buffer.len().saturating_sub(1);
            if self.buffer.len() > EVENT_BYTES {
                return Err(Error::InvalidResponse(None));
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
    let text =
        std::str::from_utf8(event).map_err(|error| Error::InvalidResponse(Some(error.into())))?;
    let data: Vec<&str> = text
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(|line| line.strip_prefix(' ').unwrap_or(line))
        .collect();
    if data.is_empty() {
        return Ok(None);
    }
    let data = data.join("\n");
    if data == DONE {
        return Ok(Some(Value::from(DONE)));
    }
    serde_json::from_str(&data)
        .map(Some)
        .map_err(|error| Error::InvalidResponse(Some(error.into())))
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
        assert_eq!(data(b"data: [DONE]\n\n").unwrap(), Some(super::DONE.into()));
    }
}
