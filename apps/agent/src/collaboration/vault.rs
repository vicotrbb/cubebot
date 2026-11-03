//! Vault tools (docs/adr/identity.md#a-vault-the-desktop-arbiter-fills). `vault_list` shows the items this agent may fill, without secrets;
//! `vault_fill` has the desktop's arbiter type one field at the keyboard focus, so the secret
//! never reaches the model; `vault_put` saves a secret the agent made, and its `secret` argument
//! is redacted before the call reaches the journal or the turn ledger.
use super::Collaboration;
use crate::tools::Running;
use cubebot_protocol::{
    runtime::{VaultFill, VaultPut},
    terminal::Outcome,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fill {
    item: String,
    field: cubebot_protocol::runtime::VaultField,
}

/// `desktop` offers `vault_fill`, which only a lane that controls the desktop can use.
pub fn definitions(desktop: bool) -> Vec<cubebot_providers::Tool> {
    let tool = cubebot_providers::Tool::strict;
    let text = |description: &str| json!({"type":"string","description":description});
    let nullable = |description: &str| json!({"type":["string","null"],"description":description});
    let mut tools = vec![
        tool(
            "vault_list",
            "List the logins and secrets in your vault: id, kind, name, site and username, never the secret. Fill one with vault_fill instead of asking a person for it.",
            json!({}),
        ),
        tool(
            "vault_put",
            "Save a password or secret you created, such as for a new account, in your own vault to fill later. An item with the same name is replaced. Only printable ASCII and spaces can be filled.",
            json!({
                "kind":{"type":"string","enum":["login","secret"],"description":"login has a site, a username and the password as secret."},
                "name":text("A short name, such as GitHub."),
                "site":nullable("A login's domain, such as github.com; null for a secret."),
                "username":nullable("A login's username; null for a secret."),
                "secret":text("The password or secret value."),
            }),
        ),
    ];
    tools.extend(desktop.then(|| tool(
        "vault_fill",
        "Type a username or secret from your vault at the desktop's keyboard focus without seeing it. Click the field first, then take a screenshot to check.",
        json!({
            "item":text("The item's id from vault_list."),
            "field":{"type":"string","enum":["username","secret"],"description":"secret is a login's password or a secret's value."},
        }),
    )));
    tools
}

/// A call's arguments as the journal and ledger keep them: unparsed `vault_put` text is dropped
/// whole, since it may hold the secret.
pub fn redacted(name: &str, mut input: Value) -> Value {
    if name != "vault_put" {
        return input;
    }
    let Some(object) = input.as_object_mut() else {
        return json!("[redacted]");
    };
    if let Some(secret) = object.get_mut("secret") {
        *secret = json!("[redacted]");
    }
    input
}

impl Collaboration {
    /// `None` when `name` is not a vault tool.
    pub(super) fn vault(
        &self,
        name: &str,
        input: &Value,
    ) -> Option<Result<Running<'_>, &'static str>> {
        let running: Running<'_> = match name {
            "vault_list" => Box::pin(self.list()),
            "vault_fill" => {
                let fill = Fill::deserialize(input)
                    .ok()
                    .and_then(|fill| Some((Uuid::parse_str(&fill.item).ok()?, fill.field)))
                    .map(|(item, field)| VaultFill { item, field });
                let Some(fill) = fill else {
                    return Some(Err(
                        "item is an id from vault_list and field is username or secret",
                    ));
                };
                Box::pin(async move {
                    let client = self.client.as_ref();
                    let sent = match client {
                        Some(client) => client.vault_fill(&fill).await,
                        None => Err(crate::control::Error::Unavailable(None)),
                    };
                    reported(
                        sent,
                        "Typed at the keyboard focus; take a screenshot to check.",
                    )
                })
            }
            "vault_put" => {
                let Ok(put) = VaultPut::deserialize(input) else {
                    return Some(Err("kind, name, site, username and secret are required"));
                };
                Box::pin(async move {
                    let sent = match &self.client {
                        Some(client) => client.vault_put(&put).await,
                        None => Err(crate::control::Error::Unavailable(None)),
                    };
                    reported(sent, &format!("Saved {} in your vault.", put.name))
                })
            }
            _ => return None,
        };
        Some(Ok(running))
    }

    async fn list(&self) -> Outcome {
        let listed = match &self.client {
            Some(client) => client.vault().await,
            None => Err(crate::control::Error::Unavailable(None)),
        };
        match listed {
            Ok(items) => Outcome::Exited {
                code: Some(0),
                output: serde_json::to_string(&items).unwrap_or_default(),
            },
            Err(_) => Outcome::Exited {
                code: None,
                output: "The orchestrator did not answer; try again.".into(),
            },
        }
    }
}

fn reported(sent: Result<Result<(), String>, crate::control::Error>, done: &str) -> Outcome {
    let (code, output) = match sent {
        Ok(Ok(())) => (Some(0), done.to_owned()),
        Ok(Err(reason)) => (Some(1), format!("Refused: {reason}.")),
        Err(_) => (
            None,
            "The orchestrator did not answer; it may or may not have happened.".into(),
        ),
    };
    Outcome::Exited { code, output }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_put_secret_is_redacted() {
        let put =
            json!({"kind":"secret","name":"key","site":null,"username":null,"secret":"hunter2"});
        let kept = redacted("vault_put", put);
        assert_eq!(kept["secret"], "[redacted]");
        assert_eq!(kept["name"], "key");
        assert_eq!(
            redacted("vault_put", json!("{\"secret\":\"hun")),
            "[redacted]"
        );
        let other = json!({"secret":"not a vault call"});
        assert_eq!(redacted("run_command", other.clone()), other);
    }
}
