//! What a connector declares: static data the console shows, the orchestrator signs in with and
//! the agent offers as tools.
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// What a tool can do. `Send` is irreversible or reaches other people: email, a chat post, a comment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Risk {
    Read,
    Write,
    Send,
}

/// A connection's permission level: the tools at or below it are offered. With no approval queue,
/// this is the safety control (docs/adr/agents.md#autonomous-tool-use-within-granted-permissions).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Read,
    /// Read and write.
    Write,
    /// Includes sending.
    Full,
}

impl Level {
    pub fn permits(self, risk: Risk) -> bool {
        match self {
            Self::Read => risk == Risk::Read,
            Self::Write => risk != Risk::Send,
            Self::Full => true,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Full => "full",
        }
    }
    pub fn parse(value: &str) -> Option<Self> {
        [Self::Read, Self::Write, Self::Full]
            .into_iter()
            .find(|level| level.as_str() == value)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum Category {
    MailCalendar,
    Files,
    Dev,
    Work,
    Chat,
    Home,
    Finance,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Manifest {
    /// Letters, digits and hyphens; stored on every connection, so it never changes.
    pub id: &'static str,
    pub name: &'static str,
    /// One line for the catalog card.
    pub description: &'static str,
    pub category: Category,
    /// The ways to sign in, the first being the one suggested.
    pub auth: &'static [Auth],
    /// Settings every sign-in method shares, such as a server address.
    pub config: &'static [Field],
    /// Parts of a suite a connection can switch on, such as Gmail and Calendar.
    pub products: &'static [Product],
    pub tools: &'static [ToolSpec],
    pub events: &'static [EventSpec],
    /// The level a new connection gets.
    pub level: Level,
    /// The setup wizard. `{redirect_uri}` in a step stands for this installation's address.
    pub guide: &'static [Step],
    /// Something the person must know before relying on the connection.
    pub warning: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Auth {
    /// A pasted token or key.
    ApiKey { fields: &'static [Field] },
    /// A username and a password, usually an app password.
    Password { fields: &'static [Field] },
    /// The person's own OAuth client (no Cubebot-registered apps). `{key}` in an endpoint is the
    /// connection's setting of that name, such as a Microsoft tenant.
    #[serde(rename = "oauth2")]
    OAuth2(OAuth2),
    /// GitHub creates an app on the person's account from a manifest, and the orchestrator mints
    /// hour-long installation tokens with its key.
    GithubApp { fields: &'static [Field] },
    /// Borrows another connection's access token, such as Email signing in to Gmail with XOAUTH2.
    Linked {
        connectors: &'static [&'static str],
        /// The product that connection needs switched on.
        product: &'static str,
    },
    /// The person's own app trading its id and secret for short-lived tokens without a browser
    /// (the client-credentials grant), such as a Zoom Server-to-Server app.
    ClientCredentials(ClientCredentials),
    /// Nothing to sign in with, such as public feeds.
    Public,
}

impl Auth {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::ApiKey { .. } => "api_key",
            Self::Password { .. } => "password",
            Self::OAuth2(_) => "oauth2",
            Self::GithubApp { .. } => "github_app",
            Self::Linked { .. } => "linked",
            Self::ClientCredentials(_) => "client_credentials",
            Self::Public => "public",
        }
    }
    pub fn fields(&self) -> &'static [Field] {
        match self {
            Self::ApiKey { fields } | Self::Password { fields } | Self::GithubApp { fields } => {
                fields
            }
            Self::OAuth2(oauth) => oauth.fields,
            Self::ClientCredentials(app) => app.fields,
            Self::Linked { .. } | Self::Public => &[],
        }
    }
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OAuth2 {
    pub authorize: &'static str,
    pub token: &'static str,
    /// A device-code endpoint: sign-in without any redirect, for public clients.
    pub device: Option<&'static str>,
    pub revoke: Option<&'static str>,
    pub scopes: &'static [Scope],
    /// Extra authorization parameters, such as Google's `access_type=offline`.
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<Vec<String>>))]
    pub params: &'static [(&'static str, &'static str)],
    /// The client id, and the secret when the client has one.
    pub fields: &'static [Field],
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ClientCredentials {
    /// The token endpoint; the client id and secret go in Basic authentication (RFC 6749 §2.3.1).
    pub token: &'static str,
    /// The form's `grant_type`: `client_credentials` unless the vendor names its own.
    pub grant: &'static str,
    /// Extra form fields; `{key}` in a value is the connection's setting of that name.
    #[cfg_attr(feature = "openapi", schema(value_type = Vec<Vec<String>>))]
    pub params: &'static [(&'static str, &'static str)],
    /// The client id, its secret (key `client_secret`) and any setting the params name.
    pub fields: &'static [Field],
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Scope {
    pub scope: &'static str,
    /// Requested only while this product is switched on; always when absent.
    pub product: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Field {
    /// The setting's name; a secret's is its name in the sealed credential.
    pub key: &'static str,
    pub label: &'static str,
    pub kind: FieldKind,
    pub required: bool,
    pub placeholder: &'static str,
    pub help: &'static str,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", content = "choices", rename_all = "snake_case")]
pub enum FieldKind {
    Text,
    /// Sealed with the credential and never shown again.
    Secret,
    /// Several lines, such as a list of allowed paths; stored as an array of strings.
    Lines,
    Url,
    Number,
    Choice(&'static [Choice]),
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Choice {
    pub value: &'static str,
    pub label: &'static str,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Product {
    pub id: &'static str,
    pub name: &'static str,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ToolSpec {
    /// Offered to agents as `<slug>__<name>`.
    pub name: &'static str,
    pub description: &'static str,
    pub risk: Risk,
    pub product: Option<&'static str>,
    /// The input's JSON Schema, an object.
    #[serde(skip)]
    pub input: &'static str,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct EventSpec {
    /// Such as `mail.received`.
    pub kind: &'static str,
    pub description: &'static str,
    pub product: Option<&'static str>,
}

#[derive(Debug, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Step {
    pub title: &'static str,
    pub body: &'static str,
    /// A page to open, such as the vendor console's credentials screen.
    pub link: Option<&'static str>,
    /// Text the person copies into the vendor's form.
    pub copy: Option<&'static str>,
}

/// The guide step of a connector whose server usually sits on a private network, such as Home
/// Assistant: agents call it from their sandbox, which reaches private addresses only when granted
/// (docs/adr/sandbox.md#public-internet-access-with-grants-for-private-destinations).
pub const PRIVATE_ADDRESS: Step = Step {
    title: "Let agents reach a server on your network",
    body: "Cubebot tests the connection from its own server, but agents call it from their sandboxes. Installed with Docker, sandboxes already reach this machine's network. Installed with the Helm chart, they reach no private address (10.x, 172.16–31.x, 192.168.x, 100.64–127.x) until you grant this server: add its address and port under sandbox.networkPolicy.privateDestinations in your values, as below, and upgrade the release. A public address needs no grant.",
    link: None,
    copy: Some("privateDestinations:\n  - cidr: 192.168.1.20/32\n    ports: [8123]"),
};

impl Manifest {
    pub fn tool(&self, name: &str) -> Option<&'static ToolSpec> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    /// The products a connection switched on: all of them unless its settings list some.
    pub fn enabled(&self, config: &Value) -> Vec<&'static str> {
        let chosen = config["products"].as_array();
        self.products
            .iter()
            .map(|product| product.id)
            .filter(|id| chosen.is_none_or(|chosen| chosen.iter().any(|item| item == id)))
            .collect()
    }

    /// The tools a connection offers at `level`, without the ones switched off.
    pub fn offered<'a>(
        &'static self,
        config: &Value,
        level: Level,
        disabled: &'a [String],
    ) -> impl Iterator<Item = &'static ToolSpec> + 'a {
        let enabled = self.enabled(config);
        self.tools.iter().filter(move |tool| {
            level.permits(tool.risk)
                && !disabled.iter().any(|name| name == tool.name)
                && tool
                    .product
                    .is_none_or(|product| enabled.contains(&product))
        })
    }

    /// The scopes a sign-in asks for, given the products switched on.
    pub fn scopes(&self, oauth: &OAuth2, config: &Value) -> Vec<&'static str> {
        let enabled = self.enabled(config);
        oauth
            .scopes
            .iter()
            .filter(|scope| {
                scope
                    .product
                    .is_none_or(|product| enabled.contains(&product))
            })
            .map(|scope| scope.scope)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_level_permits_what_is_at_or_below_it() {
        assert!(Level::Read.permits(Risk::Read));
        assert!(!Level::Read.permits(Risk::Write));
        assert!(Level::Write.permits(Risk::Write));
        assert!(!Level::Write.permits(Risk::Send));
        assert!(Level::Full.permits(Risk::Send));
        assert_eq!(Level::parse("write"), Some(Level::Write));
        assert_eq!(Level::parse("admin"), None);
    }
}
