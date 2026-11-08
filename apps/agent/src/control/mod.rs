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
        let issued: RuntimeSession = transport::decode(response, 20 * 1024).await?;
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
fn accepted(response: Result<reqwest::Response, String>) -> Result<Result<(), String>, Error> {
    match response {
        Ok(response) if response.status().is_success() => Ok(Ok(())),
        Ok(_) => Err(Error::Unavailable(None)),
        Err(reason) => Ok(Err(reason)),
    }
}
async fn refused(
    request: reqwest::RequestBuilder,
) -> Result<Result<reqwest::Response, String>, Error> {
    let response = request.send().await.map_err(request_failed)?;
    let status = response.status();
    if status.is_success() {
        return Ok(Ok(response));
    }
    // A full installation quota is a refusal, not an unknown outcome.
    let full = status == reqwest::StatusCode::INSUFFICIENT_STORAGE;
    if !(status.is_client_error() || full) || status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(Error::Unavailable(None));
    }
    Ok(Err(transport::refusal(response, status.to_string()).await))
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
        transport::decode(response, 256 * 1024).await
    }

    #[tracing::instrument(skip_all, fields(run = %run))]
    pub async fn start_dream(&self, run: Uuid) -> Result<Option<runtime::Dream>, Error> {
        let response = self.dream(reqwest::Method::POST, run, "")?.send().await;
        let response = response.map_err(request_failed)?;
        if response.status() == reqwest::StatusCode::CONFLICT {
            return Ok(None);
        }
        // Fifty memories of 4 KiB, twenty exchanges and twenty 64 KiB skills, JSON-escaped.
        transport::decode(response, 8 * 1024 * 1024).await.map(Some)
    }

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
        transport::decode(response, 512 * 1024).await.map(Some)
    }

    /// The agent's current provider material; a subscription's access token is renewed here.
    #[tracing::instrument(skip_all)]
    pub async fn provider(&self) -> Result<runtime::ProviderKey, Error> {
        let url = self.url.join("provider").map_err(Error::configuration)?;
        let response = self.http.get(url).bearer_auth(&self.token).send().await;
        transport::decode(response.map_err(request_failed)?, 64 * 1024).await
    }

    /// A connection's short-lived credential, or the refusal's status: 404 when this agent may no
    /// longer use it, 409 when it needs signing in again. Never log it.
    #[tracing::instrument(skip_all, fields(connection = %connection))]
    pub async fn connection_credential(
        &self,
        connection: Uuid,
    ) -> Result<Result<runtime::ConnectionCredential, u16>, Error> {
        let url = self
            .url
            .join(&format!("connections/{connection}/credential"));
        let url = url.map_err(Error::configuration)?;
        let response = self.http.get(url).bearer_auth(&self.token).send().await;
        let response = response.map_err(request_failed)?;
        match response.status().as_u16() {
            status @ (404 | 409) => Ok(Err(status)),
            _ => transport::decode(response, 64 * 1024).await.map(Ok),
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn wallpaper(&self) -> Result<Option<Vec<u8>>, Error> {
        let url = self.url.join("wallpaper").map_err(Error::configuration)?;
        let response = self.http.get(url).bearer_auth(&self.token).send().await;
        let response = response.map_err(request_failed)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        transport::bytes(response, 4 * 1024 * 1024).await.map(Some)
    }

    #[tracing::instrument(skip_all, fields(turn = %turn))]
    pub async fn collaborate(
        &self,
        turn: Uuid,
        request: &runtime::Collaborate,
    ) -> Result<Result<Uuid, String>, Error> {
        match self
            .post(&format!("turns/{turn}/messages"), request)
            .await?
        {
            Ok(response) => {
                let created: runtime::Collaborated = transport::decode(response, 20 * 1024).await?;
                Ok(Ok(created.turn))
            }
            Err(reason) => Ok(Err(reason)),
        }
    }

    #[tracing::instrument(skip_all, fields(turn = %turn))]
    pub async fn request_login(
        &self,
        turn: Uuid,
        request: &runtime::RequestLogin,
    ) -> Result<Result<(), String>, Error> {
        accepted(self.post(&format!("turns/{turn}/logins"), request).await?)
    }

    #[tracing::instrument(skip_all, fields(turn = %turn))]
    pub async fn broadcast(
        &self,
        turn: Uuid,
        request: &runtime::Broadcast,
    ) -> Result<Result<u32, String>, Error> {
        match self
            .post(&format!("turns/{turn}/broadcasts"), request)
            .await?
        {
            Ok(response) => {
                let sent: runtime::Broadcasted = transport::decode(response, 1024).await?;
                Ok(Ok(sent.agents))
            }
            Err(reason) => Ok(Err(reason)),
        }
    }

    #[tracing::instrument(skip_all, fields(turn = %turn))]
    pub async fn routine(
        &self,
        turn: Uuid,
        call: &runtime::RoutineCall,
    ) -> Result<Result<String, String>, Error> {
        match self.post(&format!("turns/{turn}/routines"), call).await? {
            Ok(response) => {
                let answer: runtime::RoutineAnswer =
                    transport::decode(response, 1024 * 1024).await?;
                Ok(Ok(answer.output))
            }
            Err(reason) => Ok(Err(reason)),
        }
    }

    #[tracing::instrument(skip_all, fields(turn = %turn))]
    pub async fn skill_tool(
        &self,
        turn: Uuid,
        call: &runtime::RoutineCall,
    ) -> Result<Result<String, String>, Error> {
        match self.post(&format!("turns/{turn}/skills"), call).await? {
            Ok(response) => {
                let answer: runtime::RoutineAnswer =
                    transport::decode(response, 1024 * 1024).await?;
                Ok(Ok(answer.output))
            }
            Err(reason) => Ok(Err(reason)),
        }
    }

    #[tracing::instrument(skip_all, fields(turn = %turn, file = %file))]
    pub async fn attach_file(&self, turn: Uuid, file: Uuid) -> Result<Result<(), String>, Error> {
        let body = runtime::AttachFile { file };
        match self
            .post(&format!("turns/{turn}/attachments"), &body)
            .await?
        {
            Ok(response) if response.status().is_success() => Ok(Ok(())),
            Ok(response) => Err(Error::Unavailable(Some(
                response.status().to_string().into(),
            ))),
            Err(reason) => Ok(Err(reason)),
        }
    }

    #[tracing::instrument(skip_all)]
    pub async fn vault(&self) -> Result<Vec<runtime::VaultItem>, Error> {
        let url = self.url.join("vault").map_err(Error::configuration)?;
        let response = self.http.get(url).bearer_auth(&self.token).send().await;
        let response = response.map_err(request_failed)?;
        transport::decode(response, 1024 * 1024).await
    }

    #[tracing::instrument(skip_all)]
    pub async fn vault_fill(
        &self,
        request: &runtime::VaultFill,
    ) -> Result<Result<(), String>, Error> {
        accepted(self.post("vault/fills", request).await?)
    }

    /// Never log `request`.
    #[tracing::instrument(skip_all)]
    pub async fn vault_put(
        &self,
        request: &runtime::VaultPut,
    ) -> Result<Result<(), String>, Error> {
        accepted(self.post("vault", request).await?)
    }

    #[tracing::instrument(skip_all)]
    pub async fn files(
        &self,
        parent: Option<Uuid>,
        shared: bool,
    ) -> Result<Result<Vec<runtime::FileEntry>, String>, Error> {
        let mut query = vec![("shared", shared.to_string())];
        query.extend(parent.map(|parent| ("parent", parent.to_string())));
        let request = self
            .files_request(reqwest::Method::GET, "files")?
            .query(&query);
        match refused(request).await? {
            Ok(response) => Ok(Ok(transport::decode(response, 4 * 1024 * 1024).await?)),
            Err(reason) => Ok(Err(reason)),
        }
    }

    /// `request` makes a resent upload return the first one's file; a body that ends in an error is never stored.
    #[tracing::instrument(skip_all, fields(request = %request))]
    pub async fn put_file(
        &self,
        parent: Option<Uuid>,
        name: &str,
        request: Uuid,
        body: reqwest::Body,
    ) -> Result<Result<runtime::FileEntry, String>, Error> {
        let mut query = vec![
            ("name", name.to_owned()),
            ("request_id", request.to_string()),
        ];
        query.extend(parent.map(|parent| ("parent", parent.to_string())));
        let request = self.files_request(reqwest::Method::POST, "files")?;
        match refused(request.query(&query).body(body)).await? {
            Ok(response) => Ok(Ok(transport::decode(response, 64 * 1024).await?)),
            Err(reason) => Ok(Err(reason)),
        }
    }

    #[tracing::instrument(skip_all, fields(file = %file))]
    pub async fn file_content(
        &self,
        file: Uuid,
    ) -> Result<Result<reqwest::Response, String>, Error> {
        let path = format!("files/{file}/content");
        refused(self.files_request(reqwest::Method::GET, &path)?).await
    }

    #[tracing::instrument(skip_all, fields(file = %file))]
    pub async fn share_file(
        &self,
        file: Uuid,
        share: &runtime::FileShare,
    ) -> Result<Result<(), String>, Error> {
        let path = format!("files/{file}/shares");
        let request = self
            .files_request(reqwest::Method::POST, &path)?
            .json(share);
        Ok(refused(request).await?.map(drop))
    }

    /// A transfer may outlast the client's short default timeout.
    fn files_request(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, Error> {
        let url = self.url.join(path).map_err(Error::configuration)?;
        let request = self.http.request(method, url).bearer_auth(&self.token);
        Ok(request.timeout(Duration::from_secs(30 * 60)))
    }

    /// The workspace as a tar stream, for the backup that asked; it may take hours at the pace
    /// the orchestrator reads it.
    #[tracing::instrument(skip_all, fields(backup))]
    pub async fn put_workspace(&self, backup: &str, body: reqwest::Body) -> Result<(), Error> {
        let url = self.url.join(&format!("backups/{backup}/workspace"));
        let request = self.http.put(url.map_err(Error::configuration)?);
        let request = request.bearer_auth(&self.token).body(body);
        let response = request
            .timeout(Duration::from_secs(24 * 60 * 60))
            .send()
            .await;
        match response.map_err(request_failed)?.status() {
            status if status.is_success() => Ok(()),
            reqwest::StatusCode::CONFLICT => Err(Error::unavailable(
                "the orchestrator no longer wants this workspace for that backup",
            )),
            _ => Err(Error::Unavailable(None)),
        }
    }

    /// The workspace a restore left for this agent, when one is waiting.
    pub async fn workspace_restore(&self) -> Result<Option<reqwest::Response>, Error> {
        let request = self.files_request(reqwest::Method::GET, "workspace/restore")?;
        let response = request
            .timeout(Duration::from_secs(24 * 60 * 60))
            .send()
            .await;
        let response = response.map_err(request_failed)?;
        match response.status() {
            // An orchestrator from before backups has nothing to restore.
            reqwest::StatusCode::NO_CONTENT | reqwest::StatusCode::NOT_FOUND => Ok(None),
            status if status.is_success() => Ok(Some(response)),
            _ => Err(Error::Unavailable(None)),
        }
    }

    pub async fn workspace_restored(&self, error: Option<String>) -> Result<(), Error> {
        let request = self.files_request(reqwest::Method::POST, "workspace/restore")?;
        let report = runtime::WorkspaceRestored { error };
        let response = request.json(&report).send().await.map_err(request_failed)?;
        match response.status().is_success() {
            true => Ok(()),
            false => Err(Error::Unavailable(None)),
        }
    }

    pub async fn kb_get(
        &self,
        path: &str,
        query: &[(&str, &str)],
    ) -> Result<Result<String, String>, Error> {
        let request = self.files_request(reqwest::Method::GET, path)?.query(query);
        Self::kb_text(request).await
    }

    pub async fn kb_send(
        &self,
        method: reqwest::Method,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<Result<String, String>, Error> {
        let mut request = self.files_request(method, path)?;
        if !body.is_null() {
            request = request.json(body);
        }
        Self::kb_text(request).await
    }

    pub async fn kb_attach(
        &self,
        page: Uuid,
        name: &str,
        request: Uuid,
        body: reqwest::Body,
    ) -> Result<Result<String, String>, Error> {
        let builder = self
            .files_request(
                reqwest::Method::POST,
                &format!("kb/pages/{page}/attachments"),
            )?
            .query(&[("name", name), ("request_id", &request.to_string())])
            .body(body);
        Self::kb_text(builder).await
    }

    pub async fn kb_attachment(
        &self,
        id: Uuid,
    ) -> Result<Result<reqwest::Response, String>, Error> {
        let request = self.files_request(reqwest::Method::GET, &format!("kb/attachments/{id}"))?;
        refused(request).await
    }

    async fn kb_text(request: reqwest::RequestBuilder) -> Result<Result<String, String>, Error> {
        match refused(request).await? {
            Ok(response) => {
                let bytes = transport::bytes(response, 8 * 1024 * 1024).await?;
                Ok(Ok(String::from_utf8_lossy(&bytes).into_owned()))
            }
            Err(reason) => Ok(Err(reason)),
        }
    }

    async fn post(
        &self,
        path: &str,
        body: &impl serde::Serialize,
    ) -> Result<Result<reqwest::Response, String>, Error> {
        let url = self.url.join(path);
        let url = url.map_err(Error::configuration)?;
        let response = self.http.post(url).bearer_auth(&self.token).json(body);
        let response = response.send().await.map_err(request_failed)?;
        if response.status() != reqwest::StatusCode::CONFLICT {
            return Ok(Ok(response));
        }
        let fallback = "this turn can no longer make this request".into();
        Ok(Err(transport::refusal(response, fallback).await))
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
