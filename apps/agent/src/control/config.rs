use super::Error;
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub struct Config {
    pub(super) origin: reqwest::Url,
    pub(super) token_file: PathBuf,
    /// A fixed bootstrap token, used instead of `token_file` by Docker sandboxes.
    pub(super) token: Option<String>,
    pub(super) root_certificates: Vec<reqwest::Certificate>,
    /// The same certificates for the channel's TLS connection.
    pub(super) root_pem: Vec<u8>,
}
impl Config {
    pub fn new(origin: &str, token_file: PathBuf) -> Result<Self, Error> {
        let origin = reqwest::Url::parse(origin).map_err(Error::configuration)?;
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
            return Err(Error::Configuration(None));
        }
        Ok(Self {
            origin,
            token_file,
            token: None,
            root_certificates: Vec::new(),
            root_pem: Vec::new(),
        })
    }
    pub fn with_root_certificate(mut self, pem: &[u8]) -> Result<Self, Error> {
        if pem.len() > 65536 {
            return Err(Error::Configuration(None));
        }
        self.root_certificates =
            reqwest::Certificate::from_pem_bundle(pem).map_err(Error::configuration)?;
        if self.root_certificates.is_empty() {
            return Err(Error::Configuration(None));
        }
        self.root_pem = pem.to_vec();
        Ok(self)
    }
    pub fn from_environment() -> Result<Self, Error> {
        let origin = std::env::var("CUBEBOT_ORCHESTRATOR_ORIGIN").map_err(Error::configuration)?;
        let config = match std::env::var("CUBEBOT_BOOTSTRAP_TOKEN") {
            Ok(token) => Self::new(&origin, PathBuf::new())?.with_token(token)?,
            Err(_) => {
                let path = std::env::var_os("CUBEBOT_BOOTSTRAP_TOKEN_FILE")
                    .ok_or(Error::Configuration(None))?;
                Self::new(&origin, path.into())?
            }
        };
        let Some(ca) = std::env::var_os("CUBEBOT_CONTROL_CA_FILE") else {
            return Ok(config);
        };
        let mut pem = Vec::new();
        std::fs::File::open(ca)
            .map_err(Error::configuration)?
            .take(65537)
            .read_to_end(&mut pem)
            .map_err(Error::configuration)?;
        config.with_root_certificate(&pem)
    }
    pub fn with_token(mut self, token: String) -> Result<Self, Error> {
        valid_token(token.as_bytes())?;
        self.token = Some(token);
        Ok(self)
    }
    pub(super) fn bootstrap_token(&self) -> Result<String, Error> {
        match &self.token {
            Some(token) => Ok(token.clone()),
            None => projected_token(&self.token_file),
        }
    }
}
pub(super) fn projected_token(path: &Path) -> Result<String, Error> {
    // Kubernetes rotates a projected token through symlinks; reopen the bounded file each time.
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(Error::credential)?
        .take(16385)
        .read_to_end(&mut bytes)
        .map_err(Error::credential)?;
    valid_token(&bytes)?;
    String::from_utf8(bytes).map_err(|error| Error::credential(error.utf8_error()))
}
fn valid_token(bytes: &[u8]) -> Result<(), Error> {
    if bytes.is_empty()
        || bytes.len() > 16384
        || bytes
            .iter()
            .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
    {
        return Err(Error::Credential(None));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fixed_token_replaces_the_token_file_and_is_validated() {
        let config = Config::new("http://127.0.0.1:1", "/missing".into()).unwrap();
        assert!(config.bootstrap_token().is_err());
        let config = config.with_token("cb_sandbox_secret".into()).unwrap();
        assert_eq!(config.bootstrap_token().unwrap(), "cb_sandbox_secret");
        let config = Config::new("http://127.0.0.1:1", "/missing".into()).unwrap();
        assert!(config.with_token("two words".into()).is_err());
    }
}
