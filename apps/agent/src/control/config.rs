use super::Error;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub struct Config {
    pub(super) origin: reqwest::Url,
    pub(super) token_file: PathBuf,
    pub(super) root_certificates: Vec<reqwest::Certificate>,
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
        Ok(Self {
            origin,
            token_file,
            root_certificates: Vec::new(),
        })
    }
    pub fn with_root_certificate(mut self, pem: &[u8]) -> Result<Self, Error> {
        if pem.len() > 65536 {
            return Err(Error::Configuration);
        }
        self.root_certificates =
            reqwest::Certificate::from_pem_bundle(pem).map_err(|_| Error::Configuration)?;
        if self.root_certificates.is_empty() {
            return Err(Error::Configuration);
        }
        Ok(self)
    }
    pub fn from_environment() -> Result<Self, Error> {
        let origin =
            std::env::var("CUBEBOT_ORCHESTRATOR_ORIGIN").map_err(|_| Error::Configuration)?;
        let path = std::env::var_os("CUBEBOT_BOOTSTRAP_TOKEN_FILE").ok_or(Error::Configuration)?;
        let config = Self::new(&origin, path.into())?;
        let Some(ca) = std::env::var_os("CUBEBOT_CONTROL_CA_FILE") else {
            return Ok(config);
        };
        let mut pem = Vec::new();
        std::fs::File::open(ca)
            .map_err(|_| Error::Configuration)?
            .take(65537)
            .read_to_end(&mut pem)
            .map_err(|_| Error::Configuration)?;
        config.with_root_certificate(&pem)
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
