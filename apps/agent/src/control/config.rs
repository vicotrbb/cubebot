use super::Error;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub struct Config {
    pub(super) origin: reqwest::Url,
    pub(super) token_file: PathBuf,
}
impl Config {
    pub fn new(origin: &str, token_file: PathBuf) -> Result<Self, Error> {
        let origin = reqwest::Url::parse(origin).map_err(|_| Error::Configuration)?;
        let loopback = origin.host_str().is_some_and(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        });
        let confidential = origin.scheme() == "https" || (origin.scheme() == "http" && loopback);
        if !confidential
            || origin.host_str().is_none()
            || !origin.username().is_empty()
            || origin.password().is_some()
            || origin.path() != "/"
            || origin.query().is_some()
            || origin.fragment().is_some()
        {
            return Err(Error::Configuration);
        }
        Ok(Self { origin, token_file })
    }
    pub fn from_environment() -> Result<Self, Error> {
        let origin =
            std::env::var("CUBEBOT_ORCHESTRATOR_ORIGIN").map_err(|_| Error::Configuration)?;
        let path = std::env::var_os("CUBEBOT_BOOTSTRAP_TOKEN_FILE").ok_or(Error::Configuration)?;
        Self::new(&origin, path.into())
    }
}
pub(super) fn projected_token(path: &Path) -> Result<String, Error> {
    // Kubernetes rotates a projected token through symlinks; reopen the bounded file each time.
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|_| Error::Credential)?
        .take(16385)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::Credential)?;
    if bytes.is_empty()
        || bytes.len() > 16384
        || bytes
            .iter()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(Error::Credential);
    }
    String::from_utf8(bytes).map_err(|_| Error::Credential)
}
