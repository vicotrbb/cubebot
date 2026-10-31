//! Confidential machine control: one bootstrap per process, then one channel connection at a
//! time. The issued credential lasts for this sandbox incarnation.
mod config;
mod transport;
use crate::Source;
pub use config::Config;
use cubebot_protocol::runtime::{self, RuntimeSession};
use std::{sync::Arc, time::Duration};
use tokio_tungstenite::{
    Connector, MaybeTlsStream, WebSocketStream,
    tungstenite::{client::IntoClientRequest, http::header, protocol::WebSocketConfig},
};
use uuid::Uuid;

pub type Channel = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

/// Each failure keeps its cause, when it has one, for `source()`; Display never shows it.
#[derive(Debug)]
pub enum Error {
    Configuration(Option<Source>),
    Credential(Option<Source>),
    Unavailable(Option<Source>),
    Rejected,
    Protocol(Option<Source>),
    Journal(Option<Source>),
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Configuration(_) => "invalid runtime configuration",
            Self::Credential(_) => "projected runtime credential unavailable",
            Self::Unavailable(_) => "runtime control unavailable",
            Self::Rejected => "runtime identity rejected",
            Self::Protocol(_) => "invalid runtime control response",
            Self::Journal(_) => "runtime journal requires recovery",
        })
    }
}
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Configuration(source)
            | Self::Credential(source)
            | Self::Unavailable(source)
            | Self::Protocol(source)
            | Self::Journal(source) => source
                .as_deref()
                .map(|source| source as &(dyn std::error::Error + 'static)),
            Self::Rejected => None,
        }
    }
}
impl Error {
    pub(crate) fn configuration(source: impl Into<Source>) -> Self {
        Self::Configuration(Some(source.into()))
    }
    pub(crate) fn credential(source: impl Into<Source>) -> Self {
        Self::Credential(Some(source.into()))
    }
    pub(crate) fn unavailable(source: impl Into<Source>) -> Self {
        Self::Unavailable(Some(source.into()))
    }
    pub(crate) fn protocol(source: impl Into<Source>) -> Self {
        Self::Protocol(Some(source.into()))
    }
    pub(crate) fn journal(source: impl Into<Source>) -> Self {
        Self::Journal(Some(source.into()))
    }
}

/// A request to the orchestrator that failed, without its URL, which can carry a search query.
fn request_failed(error: reqwest::Error) -> Error {
    Error::unavailable(error.without_url())
}

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
        let http = builder.build().map_err(Error::configuration)?;
        Ok(Self {
            config,
            http,
            instance: Uuid::new_v4(),
            token: None,
        })
    }
    #[tracing::instrument(skip_all, fields(instance = %self.instance))]
    pub async fn bootstrap(&mut self) -> Result<(), Error> {
        let token = self.config.bootstrap_token()?;
        let response = self
            .http
            .post(
                self.config
                    .origin
                    .join("api/runtime/bootstrap")
                    .map_err(Error::configuration)?,
            )
            .json(&runtime::Bootstrap {
                token,
                instance: self.instance,
            })
            .send()
            .await
            .map_err(request_failed)?;
        let issued: RuntimeSession = transport::decode(response).await?;
        validate_session(&issued)?;
        self.token = Some(issued.token);
        Ok(())
    }
    #[tracing::instrument(skip_all, fields(instance = %self.instance))]
    pub async fn connect(&self) -> Result<Channel, Error> {
        let token = self.token.as_deref().ok_or(Error::Credential(None))?;
        let mut url = self
            .config
            .origin
            .join("api/runtime/channel")
            .map_err(Error::configuration)?;
        let tls = url.scheme() == "https";
        url.set_scheme(if tls { "wss" } else { "ws" })
            .map_err(|()| Error::Configuration(None))?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(Error::configuration)?;
        request.headers_mut().insert(
            header::AUTHORIZATION,
            format!("Bearer {token}")
                .parse()
                .map_err(Error::credential)?,
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
        .map_err(Error::unavailable)?
        .map_err(|error| match error {
            tokio_tungstenite::tungstenite::Error::Http(response)
                if matches!(response.status().as_u16(), 401 | 403) =>
            {
                Error::Rejected
            }
            error => Error::unavailable(error),
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
                .add(certificate.map_err(Error::configuration)?)
                .map_err(Error::configuration)?;
        }
        Ok(rustls::ClientConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .map_err(Error::configuration)?
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
    #[tracing::instrument(skip_all)]
    pub async fn search(&self, query: &str) -> Result<Vec<runtime::Memory>, Error> {
        let response = self
            .http
            .get(self.url.clone())
            .query(&[("query", query)])
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(request_failed)?;
        // Ten results of at most 4 KiB each, JSON-escaped.
        transport::decode_bounded(response, 256 * 1024).await
    }

    /// `None` when the orchestrator does not admit the run.
    #[tracing::instrument(skip_all, fields(run = %run))]
    pub async fn start_dream(&self, run: Uuid) -> Result<Option<runtime::Dream>, Error> {
        let response = self.dream(reqwest::Method::POST, run, "")?.send().await;
        let response = response.map_err(request_failed)?;
        if response.status() == reqwest::StatusCode::CONFLICT {
            return Ok(None);
        }
        // Fifty memories of 4 KiB, twenty exchanges and twenty 64 KiB skills, JSON-escaped.
        transport::decode_bounded(response, 8 * 1024 * 1024)
            .await
            .map(Some)
    }

    /// `false` once the run has ended, so the change was not committed.
    #[tracing::instrument(skip_all, fields(run = %run))]
    pub async fn commit_dream(
        &self,
        run: Uuid,
        change: &runtime::MemoryChange,
    ) -> Result<bool, Error> {
        let response = self.dream(reqwest::Method::POST, run, "/changes")?;
        let response = response.json(change).send().await;
        match response.map_err(request_failed)?.status().as_u16() {
            204 => Ok(true),
            409 => Ok(false),
            _ => Err(Error::Unavailable(None)),
        }
    }

    /// `false` once the run has ended, so the skill was not published.
    #[tracing::instrument(skip_all, fields(run = %run))]
    pub async fn commit_dream_skill(
        &self,
        run: Uuid,
        change: &runtime::SkillChange,
    ) -> Result<bool, Error> {
        let response = self.dream(reqwest::Method::POST, run, "/skills")?;
        let response = response.json(change).send().await;
        match response.map_err(request_failed)?.status().as_u16() {
            // A package the library refuses is dropped like a stale one; the run goes on.
            204 | 400 => Ok(true),
            409 => Ok(false),
            _ => Err(Error::Unavailable(None)),
        }
    }

    /// This agent's installed revision of a skill; `None` when it is not installed.
    #[tracing::instrument(skip_all, fields(skill = name))]
    pub async fn skill(&self, name: &str) -> Result<Option<runtime::SkillPackage>, Error> {
        // Relative to `.../api/runtime/memories`, so this names `.../api/runtime/skills/{name}`.
        let url = self.url.join(&format!("skills/{name}"));
        let url = url.map_err(Error::configuration)?;
        let response = self.http.get(url).bearer_auth(&self.token).send().await;
        let response = response.map_err(request_failed)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        // One package of at most 64 KiB, JSON-escaped.
        transport::decode_bounded(response, 512 * 1024)
            .await
            .map(Some)
    }

    /// The created child, or the orchestrator's reason for refusing it.
    #[tracing::instrument(skip_all, fields(turn = %turn))]
    pub async fn collaborate(
        &self,
        turn: Uuid,
        request: &runtime::Collaborate,
    ) -> Result<Result<Uuid, String>, Error> {
        // Relative to `.../api/runtime/memories`, so this names `.../api/runtime/turns/...`.
        let url = self.url.join(&format!("turns/{turn}/messages"));
        let url = url.map_err(Error::configuration)?;
        let response = self.http.post(url).bearer_auth(&self.token).json(request);
        let response = response.send().await.map_err(request_failed)?;
        if response.status() == reqwest::StatusCode::CONFLICT {
            #[derive(serde::Deserialize)]
            struct Refused {
                error: String,
            }
            let refused: Result<Refused, _> = transport::decode_conflict(response).await;
            return Ok(Err(refused.map_or_else(
                |_| "this turn can no longer send work".into(),
                |refused| refused.error,
            )));
        }
        let created: runtime::Collaborated = transport::decode(response).await?;
        Ok(Ok(created.turn))
    }

    #[tracing::instrument(skip_all, fields(run = %run))]
    pub async fn finish_dream(&self, run: Uuid, end: &runtime::DreamEnd) -> Result<(), Error> {
        let response = self.dream(reqwest::Method::PUT, run, "")?;
        let response = response.json(end).send().await;
        match response.map_err(request_failed)?.status().as_u16() {
            204 | 409 => Ok(()),
            _ => Err(Error::Unavailable(None)),
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
        let url = url.map_err(Error::configuration)?;
        Ok(self.http.request(method, url).bearer_auth(&self.token))
    }
}

fn validate_session(issued: &RuntimeSession) -> Result<(), Error> {
    let secret = issued
        .token
        .strip_prefix("cb_runtime_")
        .ok_or(Error::Protocol(None))?;
    if secret.len() != 64
        || !secret.bytes().all(|byte| byte.is_ascii_hexdigit())
        || issued.agent.is_nil()
        || issued.attempt.is_nil()
        || issued.epoch < 1
        || issued.version < 1
    {
        return Err(Error::Protocol(None));
    }
    Ok(())
}
