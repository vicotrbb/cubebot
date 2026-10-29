//! The control channel between `cubebot-sandbox` and the init of its `kvm` guest (ADR 0084): one
//! vsock stream the guest opens to `HOST_CID:PORT`. The host first sends the agent's environment
//! as `NAME=value` entries, each ending in NUL, then an empty entry. A `*_FILE` entry carries the
//! named file's contents, which the guest writes to a private file of its own. Later messages in
//! the same framing carry only the `*_FILE` entries whose files changed, such as a refreshed
//! projected token. The guest writes one [`Report`] JSON line per report: liveness while the agent
//! runs, then the agent's exit status. The host ending its side stops the guest.
use serde::{Deserialize, Serialize};

pub const HOST_CID: u32 = 2;
pub const GUEST_CID: u32 = 3;
pub const PORT: u32 = 1024;
/// Bounds the environment a guest accepts, and each liveness line the host accepts.
pub const ENVIRONMENT_LIMIT: usize = 1 << 20;
pub const LINE_LIMIT: usize = 4096;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum Report {
    Liveness(crate::Liveness),
    /// The agent's exit status, or 128 plus the signal that ended it.
    Exit {
        exit: u8,
    },
}

pub fn encode(environment: &[(String, String)]) -> Result<Vec<u8>, &'static str> {
    let mut bytes = Vec::new();
    for (name, value) in environment {
        if name.is_empty() || name.contains(['=', '\0']) || value.contains('\0') {
            return Err("guest environment entry cannot be encoded");
        }
        bytes.extend_from_slice(format!("{name}={value}\0").as_bytes());
    }
    bytes.push(0);
    if bytes.len() > ENVIRONMENT_LIMIT {
        return Err("guest environment exceeds its limit");
    }
    Ok(bytes)
}

/// Whether `bytes` hold a whole message. Entries are never empty, so only the end has two NULs.
pub fn complete(bytes: &[u8]) -> bool {
    bytes == b"\0" || bytes.ends_with(b"\0\0")
}

/// Decodes a complete message: every entry up to and including the empty one.
pub fn decode(bytes: &[u8]) -> Option<Vec<(String, String)>> {
    if bytes == b"\0" {
        return Some(Vec::new());
    }
    bytes
        .strip_suffix(b"\0\0")?
        .split(|&byte| byte == 0)
        .map(|entry| {
            let (name, value) = std::str::from_utf8(entry).ok()?.split_once('=')?;
            (!name.is_empty()).then(|| (name.to_owned(), value.to_owned()))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn environments_round_trip_with_multiline_values() {
        let environment = vec![
            ("CUBEBOT_A".to_owned(), "one".to_owned()),
            (
                "CUBEBOT_CA_FILE".to_owned(),
                "-----BEGIN\nx=y\n-----END\n".to_owned(),
            ),
            ("CUBEBOT_EMPTY".to_owned(), String::new()),
        ];
        let bytes = encode(&environment).unwrap();
        assert!(complete(&bytes) && !complete(&bytes[..bytes.len() - 1]));
        assert_eq!(decode(&bytes).unwrap(), environment);
        let empty = encode(&[]).unwrap();
        assert!(complete(&empty));
        assert_eq!(decode(&empty).unwrap(), vec![]);
    }

    #[test]
    fn rejects_unencodable_and_incomplete_messages() {
        assert!(encode(&[("A=B".to_owned(), String::new())]).is_err());
        assert!(encode(&[("A".to_owned(), "\0".to_owned())]).is_err());
        assert!(decode(b"A=1\0").is_none());
        assert!(decode(b"novalue\0\0").is_none());
        assert!(decode(b"=1\0\0").is_none());
    }

    #[test]
    fn reports_are_liveness_or_an_exit_status() {
        let alive: Report = serde_json::from_str(r#"{"status":"alive"}"#).unwrap();
        assert!(matches!(alive, Report::Liveness(_)));
        let exit = serde_json::to_string(&Report::Exit { exit: 143 }).unwrap();
        assert_eq!(exit, r#"{"exit":143}"#);
        assert!(serde_json::from_str::<Report>(r#"{"exit":256}"#).is_err());
    }
}
