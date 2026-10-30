//! Confidential machine control: one bootstrap per process, then one channel connection at a
//! time. The issued credential lasts for this sandbox incarnation.
mod config;
mod transport;
pub use config::Config;
use cubebot_protocol::runtime::{self, RuntimeSession};
use std::{sync::Arc, time::Duration};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream,
    tungstenite::{client::IntoClientRequest, http::header, protocol::WebSocketConfig},
};
use uuid::Uuid;

pub type Channel = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

#[derive(Debug)]
pub enum Error {
    Configuration,
    Credential,
    Unavailable,
    Rejected,
    Protocol,
    Journal,
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Configuration => "invalid runtime configuration",
            Self::Credential => "projected runtime credential unavailable",
            Self::Unavailable => "runtime control unavailable",
            Self::Rejected => "runtime identity rejected",
            Self::Protocol => "invalid runtime control response",
            Self::Journal => "runtime journal requires recovery",
        })
    }
}
impl std::error::Error for Error {}

pub struct RuntimeClient {
    config: Config,
    http: reqwest::Client,
    instance: Uuid,
    token: Option<String>,
}
impl RuntimeClient {
    pub fn new(config: Config) -> Result<Self, Error> {
        let mut builder = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .retry(reqwest::retry::never())
            .referer(false)
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(10));
        for certificate in &config.root_certificates {
            builder = builder.add_root_certificate(certificate.clone());
        }
        let http = builder.build().map_err(|_| Error::Configuration)?;
        Ok(Self {
            config,
            http,
            instance: Uuid::new_v4(),
            token: None,
        })
    }
    pub async fn bootstrap(&mut self) -> Result<(), Error> {
        let token = self.config.bootstrap_token()?;
        let response = self
            .http
            .post(
                self.config
                    .origin
                    .join("api/runtime/bootstrap")
                    .map_err(|_| Error::Configuration)?,
            )
            .json(&runtime::Bootstrap {
                token,
                instance: self.instance,
            })
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        let issued: RuntimeSession = transport::decode(response).await?;
        validate_session(&issued)?;
        self.token = Some(issued.token);
        Ok(())
    }
    pub async fn connect(&self) -> Result<Channel, Error> {
        let token = self.token.as_deref().ok_or(Error::Credential)?;
        let mut url = self
            .config
            .origin
            .join("api/runtime/channel")
            .map_err(|_| Error::Configuration)?;
        let tls = url.scheme() == "https";
        url.set_scheme(if tls { "wss" } else { "ws" })
            .map_err(|_| Error::Configuration)?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|_| Error::Configuration)?;
        request.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Bearer {token}")
                .parse()
                .map_err(|_| Error::Credential)?,
        );
        let connector = if tls {
            Connector::Rustls(Arc::new(self.tls()?))
        } else {
            Connector::Plain
        };
        // A turn carries every exchange since the last summary, bounded by the model window.
        let config = WebSocketConfig::default()
            .max_message_size(Some(64 * 1024 * 1024))
            .max_frame_size(Some(64 * 1024 * 1024));
        let (channel, _) = tokio::time::timeout(
            Duration::from_secs(10),
            tokio_tungstenite::connect_async_tls_with_config(
                request,
                Some(config),
                false,
                Some(connector),
            ),
        )
        .await
        .map_err(|_| Error::Unavailable)?
        .map_err(|error| match error {
            tokio_tungstenite::tungstenite::Error::Http(response)
                if matches!(response.status().as_u16(), 401 | 403) =>
            {
                Error::Rejected
            }
            _ => Error::Unavailable,
        })?;
        Ok(channel)
    }
    /// Memory search for this sandbox's credential, once bootstrapped.
    pub fn memory(&self) -> Option<MemoryClient> {
        Some(MemoryClient {
            http: self.http.clone(),
            url: self.config.origin.join("api/runtime/memories").ok()?,
            token: self.token.clone()?,
        })
    }
    fn tls(&self) -> Result<rustls::ClientConfig, Error> {
        use rustls::pki_types::{CertificateDer, pem::PemObject};
        let mut roots = rustls::RootCertStore {
            roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
        };
        for certificate in CertificateDer::pem_slice_iter(&self.config.root_pem) {
            roots
                .add(certificate.map_err(|_| Error::Configuration)?)
                .map_err(|_| Error::Configuration)?;
        }
        Ok(rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(|_| Error::Configuration)?
        .with_root_certificates(roots)
        .with_no_client_auth())
    }
}
#[derive(Clone)]
pub struct MemoryClient {
    http: reqwest::Client,
    url: reqwest::Url,
    token: String,
}
impl MemoryClient {
    pub async fn search(&self, query: &str) -> Result<Vec<runtime::Memory>, Error> {
        let response = self
            .http
            .get(self.url.clone())
            .query(&[("query", query)])
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        // Ten results of at most 4 KiB each, JSON-escaped.
        transport::decode_bounded(response, 256 * 1024).await
    }

    /// `None` when the orchestrator does not admit the run.
    pub async fn start_dream(&self, run: Uuid) -> Result<Option<runtime::Dream>, Error> {
        let response = self.dream(reqwest::Method::POST, run, "")?.send().await;
        let response = response.map_err(|_| Error::Unavailable)?;
        if response.status() == reqwest::StatusCode::CONFLICT {
            return Ok(None);
        }
        // Fifty memories of 4 KiB, twenty exchanges and twenty 64 KiB skills, JSON-escaped.
        transport::decode_bounded(response, 8 * 1024 * 1024)
            .await
            .map(Some)
    }

    /// `false` once the run has ended, so the change was not committed.
    pub async fn commit_dream(
        &self,
        run: Uuid,
        change: &runtime::MemoryChange,
    ) -> Result<bool, Error> {
        let response = self.dream(reqwest::Method::POST, run, "/changes")?;
        let response = response.json(change).send().await;
        match response.map_err(|_| Error::Unavailable)?.status().as_u16() {
            204 => Ok(true),
            409 => Ok(false),
            _ => Err(Error::Unavailable),
        }
    }

    /// `false` once the run has ended, so the skill was not published.
    pub async fn commit_dream_skill(
        &self,
        run: Uuid,
        change: &runtime::SkillChange,
    ) -> Result<bool, Error> {
        let response = self.dream(reqwest::Method::POST, run, "/skills")?;
        let response = response.json(change).send().await;
        match response.map_err(|_| Error::Unavailable)?.status().as_u16() {
            // A package the library refuses is dropped like a stale one; the run goes on.
            204 | 400 => Ok(true),
            409 => Ok(false),
            _ => Err(Error::Unavailable),
        }
    }

    /// This agent's installed revision of a skill; `None` when it is not installed.
    pub async fn skill(&self, name: &str) -> Result<Option<runtime::SkillPackage>, Error> {
        // Relative to `.../api/runtime/memories`, so this names `.../api/runtime/skills/{name}`.
        let url = self.url.join(&format!("skills/{name}"));
        let url = url.map_err(|_| Error::Configuration)?;
        let response = self.http.get(url).bearer_auth(&self.token).send().await;
        let response = response.map_err(|_| Error::Unavailable)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        // One package of at most 64 KiB, JSON-escaped.
        transport::decode_bounded(response, 512 * 1024)
            .await
            .map(Some)
    }

    pub async fn finish_dream(&self, run: Uuid, end: &runtime::DreamEnd) -> Result<(), Error> {
        let response = self.dream(reqwest::Method::PUT, run, "")?;
        let response = response.json(end).send().await;
        match response.map_err(|_| Error::Unavailable)?.status().as_u16() {
            204 | 409 => Ok(()),
            _ => Err(Error::Unavailable),
        }
    }

    fn dream(
        &self,
        method: reqwest::Method,
        run: Uuid,
        suffix: &str,
    ) -> Result<reqwest::RequestBuilder, Error> {
        // Relative to `.../api/runtime/memories`, so this names `.../api/runtime/dreams/{run}`.
        let url = self.url.join(&format!("dreams/{run}{suffix}"));
        let url = url.map_err(|_| Error::Configuration)?;
        Ok(self.http.request(method, url).bearer_auth(&self.token))
    }
}

fn validate_session(issued: &RuntimeSession) -> Result<(), Error> {
    let secret = issued
        .token
        .strip_prefix("cb_runtime_")
        .ok_or(Error::Protocol)?;
    if secret.len() != 64
        || !secret.bytes().all(|byte| byte.is_ascii_hexdigit())
        || issued.agent.is_nil()
        || issued.attempt.is_nil()
        || issued.epoch < 1
        || issued.version < 1
    {
        return Err(Error::Protocol);
    }
    Ok(())
}
