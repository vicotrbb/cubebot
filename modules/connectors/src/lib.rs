//! Native connections (docs/adr/identity.md#native-connectors). A connector is compiled-in code
//! that declares how it authenticates and which tools it offers; a connection is one account on it.
//! The orchestrator signs in, refreshes, checks and polls; the agent runs the tools. A connector
//! never implements OAuth itself, and adding one is a file here plus a line in [`registry`].
mod context;
#[cfg(test)]
mod contract;
mod manifest;
mod registry;
mod settings;
#[cfg(test)]
mod testing;

pub mod airtable;
pub mod asana;
pub mod atlassian;
pub mod cal_com;
pub mod calendly;
pub mod dav;
pub mod discord;
pub mod dropbox;
pub mod email;
pub mod github;
pub mod google;
pub mod gotify;
pub mod home_assistant;
pub mod http_api;
pub mod hubspot;
pub mod linear;
pub mod microsoft;
pub mod nextcloud;
pub mod notion;
pub mod ntfy;
pub mod paperless;
pub mod pushover;
pub mod readwise;
pub mod rss;
pub mod s3;
pub mod sftp;
pub mod slack;
pub mod sql;
pub mod stripe;
pub mod telegram;
pub mod todoist;
pub mod trello;
pub mod zoom;

pub use context::{ATTACH, Attachment, Context, Credential, FILE, LARGEST, file, input};
pub use manifest::{
    Auth, Category, Choice, ClientCredentials, EventSpec, Field, FieldKind, Level, Manifest,
    OAuth2, PRIVATE_ADDRESS, Product, Risk, Scope, Step, ToolSpec,
};
pub use registry::{all, find};
pub use settings::{problem, slug_valid};

use serde::Serialize;
use serde_json::Value;
use std::{future::Future, pin::Pin};

/// A connector's asynchronous answer; boxed so connectors stay object-safe in the registry.
pub type Answer<'a, T> = Pin<Box<dyn Future<Output = Result<T, Error>> + Send + 'a>>;

pub trait Connector: Send + Sync {
    fn manifest(&self) -> &'static Manifest;
    /// "Who am I": proves the credential works and names the account, such as an email address.
    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account>;
    /// Runs one of the manifest's tools. Its result is untrusted vendor data.
    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value>;
    /// New events since `cursor`. Without a cursor it answers the current one and no events, so a
    /// new connection never replays its history. Only connectors that declare events are polled.
    fn poll<'a>(&'a self, _cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        let cursor = cursor.map(str::to_owned);
        Box::pin(async move {
            Ok(Poll {
                events: Vec::new(),
                cursor,
            })
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Account {
    /// The account as its vendor names it: an email address, a login or a workspace.
    pub name: String,
}

#[derive(Debug, Default)]
pub struct Poll {
    pub events: Vec<Event>,
    pub cursor: Option<String>,
}

/// Something that happened in the account. Its text and data are untrusted vendor content.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Event {
    /// One of the manifest's event kinds, such as `mail.received`.
    pub kind: &'static str,
    /// The vendor's id for what happened, so a repeated poll is recognised.
    pub id: String,
    pub summary: String,
    pub data: Value,
}

/// What went wrong, worded for the model that made the call. Vendor messages are untrusted and
/// never carry a URL or credential.
#[derive(Debug, PartialEq)]
pub enum Error {
    /// The vendor refused the credential: the connection needs signing in again.
    Unauthorized,
    /// The account or its granted scopes do not allow this.
    Forbidden(String),
    NotFound(String),
    /// The tool's input is not valid.
    Invalid(String),
    /// The vendor refused the request.
    Rejected(String),
    /// Nothing was done: the vendor could not be reached or failed before acting.
    Unavailable(String),
    /// The request was sent but no answer arrived, so it may have taken effect.
    Uncertain,
    UnknownTool,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unauthorized => f.write_str(
                "the account refused the connection's credential; an administrator must sign in again",
            ),
            Self::Forbidden(message) => write!(f, "not permitted: {message}"),
            Self::NotFound(message) => write!(f, "not found: {message}"),
            Self::Invalid(message) => write!(f, "invalid input: {message}"),
            Self::Rejected(message) => write!(f, "refused: {message}"),
            Self::Unavailable(message) => write!(f, "unavailable, nothing was done: {message}"),
            Self::Uncertain => {
                f.write_str("no answer arrived; the request may or may not have taken effect")
            }
            Self::UnknownTool => f.write_str("this connector has no such tool"),
        }
    }
}

impl std::error::Error for Error {}
