//! Bounded confidential machine control. Identity never grants permission to execute effects.
mod config;
mod transport;
pub use config::Config;
use cubebot_protocol::runtime::{self, RuntimeActor, RuntimeControl, RuntimeSession};
use std::time::{Duration, Instant};
use uuid::Uuid;

#[derive(Debug)]
pub enum Error {
    Configuration,
    Credential,
    Unavailable,
    Rejected,
    Protocol,
    Expired,
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
            Self::Expired => "runtime identity expired",
            Self::Journal => "runtime journal requires recovery",
        })
    }
}
impl std::error::Error for Error {}

struct Session {
    token: String,
    expires: Instant,
    renew: Instant,
}
pub struct RuntimeClient {
    config: Config,
    http: reqwest::Client,
    instance: Uuid,
    actor: Option<RuntimeActor>,
    session: Option<Session>,
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
            actor: None,
            session: None,
        })
    }
    pub(crate) fn invalidate(&mut self) {
        self.session = None;
    }

    pub fn requires_bootstrap(&self) -> bool {
        self.session
            .as_ref()
            .is_none_or(|session| Instant::now() >= session.renew)
    }
    pub async fn bootstrap(&mut self) -> Result<(), Error> {
        self.session = None;
        let token = config::projected_token(&self.config.token_file)?;
        let start = Instant::now();
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
        let actor = validate_session(&issued, self.actor.as_ref())?;
        let expires = start + Duration::from_secs(issued.expires_in_seconds);
        if Instant::now() >= expires {
            return Err(Error::Expired);
        }
        self.session = Some(Session {
            token: issued.token,
            expires,
            renew: start + Duration::from_secs(issued.expires_in_seconds / 2),
        });
        self.actor = Some(actor);
        Ok(())
    }
    pub async fn control(&self) -> Result<RuntimeControl, Error> {
        let response = self
            .request(reqwest::Method::GET, "api/runtime/control")?
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        let control: RuntimeControl = transport::decode(response).await?;
        self.check_actor(&control.actor)?;
        match (&control.disposition, &control.drain) {
            (runtime::RuntimeDisposition::Drain, Some(request)) if !request.request_id.is_nil() => {
            }
            (runtime::RuntimeDisposition::Available | runtime::RuntimeDisposition::Hold, None) => {}
            _ => return Err(Error::Protocol),
        }
        Ok(control)
    }
    pub async fn acknowledge_drain(
        &self,
        request_id: Uuid,
    ) -> Result<runtime::DrainRequest, Error> {
        let response = self
            .request(reqwest::Method::POST, "api/runtime/drained")?
            .json(&runtime::DrainAcknowledgement { request_id })
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        let report: runtime::DrainRequest = transport::decode(response).await?;
        self.valid_session()?;
        if report.request_id != request_id || !report.acknowledged {
            return Err(Error::Protocol);
        }
        Ok(report)
    }
    pub async fn synchronize_record(&self, sequence: u64, payload: &[u8]) -> Result<(), Error> {
        use sha2::{Digest, Sha256};
        if sequence == 0
            || sequence > i64::MAX as u64
            || payload.len() > crate::journal::MAX_PAYLOAD
        {
            return Err(Error::Protocol);
        }
        let response = self
            .request(reqwest::Method::POST, "api/runtime/journal")?
            .json(&runtime::JournalRecord {
                sequence,
                payload: payload.to_vec(),
            })
            .send()
            .await
            .map_err(|_| Error::Unavailable)?;
        let receipt: runtime::JournalReceipt = transport::decode(response).await?;
        self.check_actor(&receipt.actor)?;
        let expected: [u8; 32] = Sha256::digest(payload).into();
        if receipt.sequence != sequence || receipt.payload_digest != expected {
            return Err(Error::Protocol);
        }
        Ok(())
    }
    fn request(
        &self,
        method: reqwest::Method,
        path: &str,
    ) -> Result<reqwest::RequestBuilder, Error> {
        let session = self.valid_session()?;
        let url = self
            .config
            .origin
            .join(path)
            .map_err(|_| Error::Configuration)?;
        Ok(self.http.request(method, url).bearer_auth(&session.token))
    }
    fn valid_session(&self) -> Result<&Session, Error> {
        self.session
            .as_ref()
            .filter(|session| Instant::now() < session.expires)
            .ok_or(Error::Expired)
    }
    fn check_actor(&self, actor: &RuntimeActor) -> Result<(), Error> {
        self.valid_session()?;
        if self.actor.as_ref() != Some(actor) {
            return Err(Error::Protocol);
        }
        Ok(())
    }
}
fn validate_session(
    issued: &RuntimeSession,
    previous: Option<&RuntimeActor>,
) -> Result<RuntimeActor, Error> {
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
        || !(1..=120).contains(&issued.expires_in_seconds)
    {
        return Err(Error::Protocol);
    }
    if let Some(old) = previous
        && (old.agent != issued.agent
            || old.attempt != issued.attempt
            || old.epoch != issued.epoch
            || old.version >= issued.version)
    {
        return Err(Error::Protocol);
    }
    Ok(RuntimeActor {
        agent: issued.agent,
        attempt: issued.attempt,
        epoch: issued.epoch,
        version: issued.version,
    })
}
