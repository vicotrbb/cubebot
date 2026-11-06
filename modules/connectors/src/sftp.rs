//! Files on an SSH server over SFTP, with russh. The server's host key must match the fingerprint
//! the person pinned, or nothing is sent: that is the trust boundary, as there is no known_hosts.
//! An optional root confines every path, symlinks included, by asking the server where it leads.
use crate::{
    Account, Answer, Attachment, Auth, Category, Connector, Context, Credential, Error, Field,
    FieldKind, LARGEST, Level, Manifest, PRIVATE_ADDRESS, Risk, Step, ToolSpec,
};
use russh::{
    client::{self, AuthResult, Handle},
    keys::{HashAlg, PrivateKeyWithHashAlg, PublicKeyOrCertificate, decode_secret_key},
};
use russh_sftp::{
    client::{SftpSession, error::Error as SftpError, fs::Metadata},
    protocol::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const LISTED: usize = 1000;
const CONNECTING: Duration = Duration::from_secs(15);
const IDLE: Duration = Duration::from_secs(60);

pub struct Sftp;

static MANIFEST: Manifest = Manifest {
    id: "sftp",
    name: "SFTP",
    description: "Browse, read, upload, move and delete files on an SSH server over SFTP.",
    category: Category::Data,
    auth: &[
        Auth::Password {
            fields: &[
                Field {
                    key: "username",
                    label: "User",
                    kind: FieldKind::Text,
                    required: true,
                    placeholder: "cubebot",
                    help: "",
                },
                Field {
                    key: "password",
                    label: "Password",
                    kind: FieldKind::Secret,
                    required: true,
                    placeholder: "",
                    help: "",
                },
            ],
        },
        Auth::ApiKey {
            fields: &[
                Field {
                    key: "username",
                    label: "User",
                    kind: FieldKind::Text,
                    required: true,
                    placeholder: "cubebot",
                    help: "",
                },
                Field {
                    key: "private_key",
                    label: "Private key",
                    kind: FieldKind::Secret,
                    required: true,
                    placeholder: "-----BEGIN OPENSSH PRIVATE KEY-----",
                    help: "The whole key file, such as ~/.ssh/cubebot: OpenSSH, PKCS#8 or PuTTY format.",
                },
                Field {
                    key: "passphrase",
                    label: "Passphrase",
                    kind: FieldKind::Secret,
                    required: false,
                    placeholder: "",
                    help: "Only when the key has one.",
                },
            ],
        },
    ],
    config: &[
        Field {
            key: "host",
            label: "Host",
            kind: FieldKind::Text,
            required: true,
            placeholder: "files.example.com",
            help: "A name or address the agents' sandboxes can reach.",
        },
        Field {
            key: "port",
            label: "Port",
            kind: FieldKind::Number,
            required: false,
            placeholder: "22",
            help: "",
        },
        Field {
            key: "fingerprint",
            label: "Host key fingerprint",
            kind: FieldKind::Text,
            required: true,
            placeholder: "SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s",
            help: "The server's SHA256 host key fingerprint; connections to any other key are refused.",
        },
        Field {
            key: "root",
            label: "Folder",
            kind: FieldKind::Text,
            required: false,
            placeholder: "/srv/shared",
            help: "Agents see only this folder, as /. The user's whole view when empty.",
        },
    ],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_dir",
            description: "A folder's entries with their type, size and modification time, up to 1000.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string","description":"The folder; the top folder when left out."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "stat",
            description: "A file's or folder's type, size, permissions and modification time.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_file",
            description: "Save a file to the workspace, up to 25 MiB.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "write_file",
            description: "Upload a workspace file to a path, refusing to replace an existing file unless overwrite is true.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string","description":"Where to save it, such as /reports/q1.csv."},"workspace_file":{"type":"string","description":"A file in your workspace."},"overwrite":{"type":"boolean"}},"required":["path","workspace_file"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "mkdir",
            description: "Make a folder; its parent must exist.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "rename",
            description: "Move or rename a file or folder. The destination must not exist.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"from":{"type":"string"},"to":{"type":"string"}},"required":["from","to"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "remove",
            description: "Delete a file or an empty folder. It cannot be recovered.",
            risk: Risk::Write,
            product: None,
            input: r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"],"additionalProperties":false}"#,
        },
    ],
    events: &[],
    level: Level::Read,
    guide: &[
        Step {
            title: "Make an account for agents",
            body: "Use an SSH account that can reach only the files agents need, ideally chrooted to an SFTP-only folder (ChrootDirectory and ForceCommand internal-sftp in sshd_config). Sign in with its password, or with a key pair made for it: run the command below, add cubebot.pub to the account's ~/.ssh/authorized_keys, and paste the private key file cubebot.",
            link: None,
            copy: Some("ssh-keygen -t ed25519 -f cubebot -C cubebot -N ''"),
        },
        Step {
            title: "Pin the server's host key",
            body: "Cubebot connects only to a server holding this key, which stops anyone in between from reading the password or the files. On a machine you trust, run the command below with your server's name (and -p for another port), and copy the SHA256: fingerprint of the ED25519 line. Best of all, run ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub on the server itself.",
            link: None,
            copy: Some("ssh-keyscan -t ed25519 files.example.com | ssh-keygen -lf -"),
        },
        PRIVATE_ADDRESS,
    ],
    warning: Some(
        "Agents act as this SSH account, and its own file permissions are what bound them; the folder setting only narrows what tools ask for. Use an account limited to the files agents need. If the server's host key changes, update the fingerprint.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Listing {
    path: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Target {
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Write {
    path: String,
    workspace_file: Attachment,
    #[serde(default)]
    overwrite: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rename {
    from: String,
    to: String,
}

impl Connector for Sftp {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let files = Files::open(cx).await?;
            let user = cx.setting("username").unwrap_or_default();
            let host = cx.setting("host").unwrap_or_default();
            let name = format!("{user}@{host}:{}", files.root.as_deref().unwrap_or("~"));
            files.close().await;
            Ok(Account { name })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            if MANIFEST.tool(tool).is_none() {
                return Err(Error::UnknownTool);
            }
            let files = Files::open(cx).await?;
            let answer = match tool {
                "list_dir" => list(&files, input).await,
                "stat" => stat(&files, input).await,
                "read_file" => read(&files, input).await,
                "write_file" => write(&files, input).await,
                "mkdir" => mkdir(&files, input).await,
                "rename" => rename(&files, input).await,
                _ => remove(&files, input).await,
            };
            files.close().await;
            answer
        })
    }
}

/// Accepts only the server whose host key has the pinned SHA256 fingerprint.
struct Pinned {
    fingerprint: String,
}

impl client::Handler for Pinned {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            return Ok(false);
        };
        Ok(key.fingerprint(HashAlg::Sha256).to_string() == self.fingerprint)
    }
}

/// The fingerprint as `SHA256:<base64>`, from it alone or from a whole `ssh-keygen -l` line.
fn fingerprint(text: &str) -> Result<String, Error> {
    let token = text
        .split_whitespace()
        .find(|word| word.starts_with("SHA256:"))
        .unwrap_or(text.trim());
    let digest = token.strip_prefix("SHA256:").unwrap_or(token);
    let valid = digest.len() == 43
        && digest
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/');
    valid.then(|| format!("SHA256:{digest}")).ok_or_else(|| {
        Error::Invalid(
            "the host key fingerprint is not a SHA256 fingerprint as ssh-keygen -l prints it"
                .into(),
        )
    })
}

/// A key pasted into a one-line field loses its line breaks; this puts them back.
// shortcut: legacy PEM keys with Proc-Type headers lose their header lines; upgrade with a
// multi-line secret field.
fn pem(text: &str) -> String {
    let text = text.trim();
    let parts: Vec<&str> = text.split("-----").collect();
    if text.contains('\n') || parts.len() != 5 {
        return text.to_owned();
    }
    let body: String = parts[2].split_whitespace().collect();
    let lines: Vec<&str> = body
        .as_bytes()
        .chunks(70)
        .map(|chunk| std::str::from_utf8(chunk).unwrap_or_default())
        .collect();
    format!(
        "-----{}-----\n{}\n-----{}-----\n",
        parts[1],
        lines.join("\n"),
        parts[3]
    )
}

/// Where tool paths lead: under the root as `/`, else as the server takes them.
fn resolve(root: Option<&str>, path: &str) -> Result<String, Error> {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." if root.is_some() => {
                if parts.pop().is_none() {
                    return Err(Error::Invalid(format!(
                        "{path} is outside the shared folder"
                    )));
                }
            }
            part => parts.push(part),
        }
    }
    let joined = parts.join("/");
    Ok(match root {
        Some(root) if joined.is_empty() => root.to_owned(),
        Some(root) => format!("{}/{joined}", root.trim_end_matches('/')),
        None if path.starts_with('/') => format!("/{joined}"),
        None if joined.is_empty() => ".".into(),
        None => path.to_owned(),
    })
}

/// A server path as agents see it: relative to the root, which is `/`.
fn shown(root: Option<&str>, path: &str) -> String {
    let Some(root) = root else {
        return path.to_owned();
    };
    let inside = path
        .strip_prefix(root.trim_end_matches('/'))
        .unwrap_or(path);
    match inside.is_empty() {
        true => "/".into(),
        false => inside.to_owned(),
    }
}

fn within(root: &str, path: &str) -> bool {
    let root = root.trim_end_matches('/');
    path == root
        || root.is_empty()
        || path
            .strip_prefix(root)
            .is_some_and(|rest| rest.starts_with('/'))
}

struct Files {
    ssh: Handle<Pinned>,
    sftp: SftpSession,
    /// The shared folder as the server resolves it, symlinks followed.
    root: Option<String>,
}

impl Files {
    async fn open(cx: &Context) -> Result<Self, Error> {
        tokio::time::timeout(CONNECTING, Self::connect(cx))
            .await
            .map_err(|_| Error::Unavailable("timed out connecting to the SSH server".into()))?
    }

    async fn connect(cx: &Context) -> Result<Self, Error> {
        let host = cx
            .setting("host")
            .ok_or_else(|| Error::Invalid("the connection has no host".into()))?
            .trim();
        let pinned = Pinned {
            fingerprint: fingerprint(cx.setting("fingerprint").unwrap_or_default())?,
        };
        let user = cx.setting("username").ok_or(Error::Unauthorized)?.trim();
        let config = client::Config {
            inactivity_timeout: Some(IDLE),
            ..Default::default()
        };
        let mut ssh = client::connect(Arc::new(config), (host, port(cx)?), pinned)
            .await
            .map_err(|error| refused(cx, error))?;
        let signed_in = sign_in(&mut ssh, cx, user).await?;
        if !matches!(signed_in, AuthResult::Success) {
            return Err(Error::Unauthorized);
        }
        let channel = ssh
            .channel_open_session()
            .await
            .map_err(|error| refused(cx, error))?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|error| refused(cx, error))?;
        let sftp = SftpSession::new(channel.into_stream())
            .await
            .map_err(|error| {
                Error::Unavailable(format!("the server did not start SFTP: {error}"))
            })?;
        let root = match cx.setting("root") {
            Some(root) => Some(
                sftp.canonicalize(root.trim())
                    .await
                    .map_err(|error| problem(error, "the shared folder"))?,
            ),
            None => None,
        };
        Ok(Self { ssh, sftp, root })
    }

    async fn close(self) {
        let _ = self.sftp.close().await;
        let _ = self
            .ssh
            .disconnect(russh::Disconnect::ByApplication, "", "en")
            .await;
    }

    /// The server path for `path`, confirmed inside the root once symlinks are followed: the
    /// path itself when it exists, else its parent.
    async fn path(&self, path: &str) -> Result<String, Error> {
        let resolved = resolve(self.root.as_deref(), path)?;
        let Some(root) = &self.root else {
            return Ok(resolved);
        };
        let real = match self.sftp.canonicalize(resolved.as_str()).await {
            Ok(real) => real,
            Err(_) => {
                let (parent, name) = resolved.rsplit_once('/').unwrap_or(("", &resolved));
                let parent = self
                    .sftp
                    .canonicalize(if parent.is_empty() { "/" } else { parent })
                    .await
                    .map_err(|error| problem(error, path))?;
                format!("{}/{name}", parent.trim_end_matches('/'))
            }
        };
        match within(root, &real) {
            true => Ok(real),
            false => Err(Error::Forbidden(format!(
                "{path} leads outside the shared folder"
            ))),
        }
    }
}

fn port(cx: &Context) -> Result<u16, Error> {
    let port = &cx.config()["port"];
    let text = port.as_str().map(str::trim).filter(|text| !text.is_empty());
    let number = match (text, port.as_u64()) {
        (Some(text), _) => text.parse().ok(),
        (None, Some(number)) => u16::try_from(number).ok(),
        (None, None) => Some(22),
    };
    number.ok_or_else(|| Error::Invalid("the port is not a number".into()))
}

async fn sign_in(ssh: &mut Handle<Pinned>, cx: &Context, user: &str) -> Result<AuthResult, Error> {
    let fields = match cx.credential() {
        Credential::Fields { fields } => fields,
        Credential::Token { .. } => return Err(Error::Unauthorized),
    };
    let answer = match (fields.get("private_key"), fields.get("password")) {
        (Some(key), _) => {
            let passphrase = fields.get("passphrase").map(String::as_str);
            let key = decode_secret_key(&pem(key), passphrase).map_err(|_| {
                Error::Invalid(
                    "the private key could not be read; check its passphrase and that it is the whole file"
                        .into(),
                )
            })?;
            let hash = ssh.best_supported_rsa_hash().await.ok().flatten().flatten();
            let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
            ssh.authenticate_publickey(user, key).await
        }
        (None, Some(password)) => ssh.authenticate_password(user, password).await,
        (None, None) => return Err(Error::Unauthorized),
    };
    answer.map_err(|error| refused(cx, error))
}

/// An SSH failure, without anything the server or the credential could have put in it.
fn refused(cx: &Context, error: russh::Error) -> Error {
    match error {
        russh::Error::UnknownKey => Error::Rejected(
            "the server's host key does not match the pinned fingerprint, so nothing was sent; if the key really changed, an administrator must update the fingerprint"
                .into(),
        ),
        russh::Error::IO(error) => Error::Unavailable(format!("could not connect: {error}")),
        russh::Error::ConnectionTimeout | russh::Error::InactivityTimeout => {
            Error::Unavailable("the SSH server stopped answering".into())
        }
        other => Error::Unavailable(cx.credential().redact(&format!("SSH failed: {other}"))),
    }
}

/// An SFTP status as an error about `what`.
fn problem(error: SftpError, what: &str) -> Error {
    match error {
        SftpError::Status(status) => match status.status_code {
            StatusCode::NoSuchFile => Error::NotFound(format!("{what} does not exist")),
            StatusCode::PermissionDenied => {
                Error::Forbidden(format!("the account may not use {what}"))
            }
            _ => Error::Rejected(format!("{what}: {}", status.error_message)),
        },
        SftpError::Timeout | SftpError::IO(_) => Error::Uncertain,
        other => Error::Rejected(format!("{what}: {other}")),
    }
}

fn kind(metadata: &Metadata) -> &'static str {
    match () {
        _ if metadata.is_dir() => "folder",
        _ if metadata.is_regular() => "file",
        _ if metadata.is_symlink() => "link",
        _ => "other",
    }
}

fn described(metadata: &Metadata) -> Value {
    let modified = metadata
        .mtime
        .map(|seconds| mail_parser::DateTime::from_timestamp(i64::from(seconds)).to_rfc3339());
    json!({
        "type": kind(metadata),
        "size": metadata.size,
        "modified": modified,
        "permissions": metadata.permissions.map(|mode| format!("{:o}", mode & 0o7777)),
    })
}

async fn list(files: &Files, input: Value) -> Result<Value, Error> {
    let asked = crate::input::<Listing>(input)?
        .path
        .unwrap_or_else(|| "/".into());
    let path = files.path(&asked).await?;
    let entries = files
        .sftp
        .read_dir(path.as_str())
        .await
        .map_err(|error| problem(error, &asked))?;
    let mut listed: Vec<Value> = entries
        .take(LISTED + 1)
        .map(|entry| {
            let mut shown = described(&entry.metadata());
            shown["name"] = json!(entry.file_name());
            shown
        })
        .collect();
    let truncated = listed.len() > LISTED;
    listed.truncate(LISTED);
    let path = shown(files.root.as_deref(), &path);
    Ok(json!({"path": path, "entries": listed, "truncated": truncated}))
}

async fn stat(files: &Files, input: Value) -> Result<Value, Error> {
    let asked = crate::input::<Target>(input)?.path;
    let path = files.path(&asked).await?;
    let metadata = files
        .sftp
        .metadata(path.as_str())
        .await
        .map_err(|error| problem(error, &asked))?;
    let mut shown = described(&metadata);
    shown["path"] = json!(self::shown(files.root.as_deref(), &path));
    Ok(shown)
}

async fn read(files: &Files, input: Value) -> Result<Value, Error> {
    let asked = crate::input::<Target>(input)?.path;
    let path = files.path(&asked).await?;
    let mut file = files
        .sftp
        .open(path.as_str())
        .await
        .map_err(|error| problem(error, &asked))?;
    let mut bytes = Vec::new();
    let reading = (&mut file)
        .take(LARGEST as u64 + 1)
        .read_to_end(&mut bytes)
        .await;
    let _ = file.close().await;
    reading.map_err(|_| Error::Unavailable(format!("{asked} could not be read")))?;
    if bytes.len() > LARGEST {
        return Err(Error::Rejected(format!("{asked} is larger than 25 MiB")));
    }
    let name = path.rsplit('/').next().unwrap_or("file");
    Ok(crate::file(name, None, &bytes))
}

async fn write(files: &Files, input: Value) -> Result<Value, Error> {
    let write: Write = crate::input(input)?;
    let path = files.path(&write.path).await?;
    let bytes = write.workspace_file.bytes()?;
    if !write.overwrite && files.sftp.try_exists(path.as_str()).await.unwrap_or(false) {
        return Err(Error::Rejected(format!(
            "{} exists; pass overwrite to replace it",
            write.path
        )));
    }
    let mut file = files
        .sftp
        .create(path.as_str())
        .await
        .map_err(|error| problem(error, &write.path))?;
    let written = file.write_all(&bytes).await;
    let closed = file.shutdown().await;
    written.and(closed).map_err(|_| Error::Uncertain)?;
    let path = shown(files.root.as_deref(), &path);
    Ok(json!({"path": path, "size": bytes.len()}))
}

async fn mkdir(files: &Files, input: Value) -> Result<Value, Error> {
    let asked = crate::input::<Target>(input)?.path;
    let path = files.path(&asked).await?;
    files
        .sftp
        .create_dir(path.as_str())
        .await
        .map_err(|error| problem(error, &asked))?;
    Ok(json!({"made": shown(files.root.as_deref(), &path)}))
}

async fn rename(files: &Files, input: Value) -> Result<Value, Error> {
    let rename: Rename = crate::input(input)?;
    let from = files.path(&rename.from).await?;
    let to = files.path(&rename.to).await?;
    files
        .sftp
        .rename(from.as_str(), to.as_str())
        .await
        .map_err(|error| problem(error, &rename.from))?;
    let root = files.root.as_deref();
    Ok(json!({"from": shown(root, &from), "to": shown(root, &to)}))
}

async fn remove(files: &Files, input: Value) -> Result<Value, Error> {
    let asked = crate::input::<Target>(input)?.path;
    let path = files.path(&asked).await?;
    if files.root.as_deref() == Some(path.as_str()) {
        return Err(Error::Forbidden(
            "the shared folder itself cannot be removed".into(),
        ));
    }
    let metadata = files
        .sftp
        .symlink_metadata(path.as_str())
        .await
        .map_err(|error| problem(error, &asked))?;
    let removed = match metadata.is_dir() {
        true => files.sftp.remove_dir(path.as_str()).await,
        false => files.sftp.remove_file(path.as_str()).await,
    };
    removed.map_err(|error| problem(error, &asked))?;
    Ok(json!({"removed": shown(files.root.as_deref(), &path)}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fingerprint_is_taken_from_ssh_keygen_output_and_anything_else_is_refused() {
        let pinned = "SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s";
        let line = format!("256 {pinned} files.example.com (ED25519)");
        assert_eq!(fingerprint(&line).as_deref(), Ok(pinned));
        assert_eq!(fingerprint(&pinned[7..]).as_deref(), Ok(pinned));
        assert_eq!(fingerprint(&format!("  {pinned}\n")).as_deref(), Ok(pinned));
        for bad in [
            "",
            "MD5:16:27:ac:a5:76:28:2d:36:63:1b:56:4d:eb:df:a6:48",
            "SHA256:short",
        ] {
            assert!(matches!(fingerprint(bad), Err(Error::Invalid(_))), "{bad}");
        }
    }

    #[test]
    fn a_host_key_matches_only_its_own_fingerprint() {
        // Made with ssh-keygen -t ed25519; ssh-keygen -lf printed its fingerprint.
        let public = russh::keys::PublicKey::from_openssh(
            "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIB3xpXo20QqW7b5VTkgp14LAJQEFidmAqSyZs1B7mQdS test",
        )
        .unwrap();
        let own =
            fingerprint("256 SHA256:3kNbrn75w5kid2i4QP8zaes+8vwOMEmSZrnTzjwFnKI test (ED25519)")
                .unwrap();
        let offered = PublicKeyOrCertificate::PublicKey {
            key: public,
            hash_alg: None,
        };
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let mut pinned = Pinned { fingerprint: own };
        assert!(
            runtime
                .block_on(client::Handler::check_server_key(&mut pinned, &offered))
                .unwrap()
        );
        let mut other = Pinned {
            fingerprint: "SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s".into(),
        };
        assert!(
            !runtime
                .block_on(client::Handler::check_server_key(&mut other, &offered))
                .unwrap()
        );
    }

    #[test]
    fn paths_stay_under_the_root_and_are_shown_relative_to_it() {
        let root = Some("/srv/shared");
        assert_eq!(
            resolve(root, "/reports/q1.csv").unwrap(),
            "/srv/shared/reports/q1.csv"
        );
        assert_eq!(
            resolve(root, "reports/./a/../q1.csv").unwrap(),
            "/srv/shared/reports/q1.csv"
        );
        assert_eq!(resolve(root, "/").unwrap(), "/srv/shared");
        assert!(matches!(
            resolve(root, "/../etc/passwd"),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(resolve(root, "a/../../b"), Err(Error::Invalid(_))));
        assert_eq!(resolve(None, "/etc/hosts").unwrap(), "/etc/hosts");
        assert_eq!(resolve(None, "notes.txt").unwrap(), "notes.txt");
        assert_eq!(resolve(None, "/").unwrap(), "/");
    }

    #[test]
    fn server_paths_are_inside_the_root_only_below_it() {
        let root = Some("/srv/shared");
        assert_eq!(shown(root, "/srv/shared/reports"), "/reports");
        assert_eq!(shown(root, "/srv/shared"), "/");
        assert!(within("/srv/shared", "/srv/shared/a"));
        assert!(within("/srv/shared", "/srv/shared"));
        assert!(!within("/srv/shared", "/srv/shared-other/a"));
        assert!(!within("/srv/shared", "/etc"));
        assert!(within("/", "/etc"));
    }

    #[test]
    fn a_key_pasted_on_one_line_gets_its_line_breaks_back() {
        let key = "-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmU=\n-----END OPENSSH PRIVATE KEY-----";
        assert_eq!(pem(key), key);
        let flattened = key.replace('\n', "");
        assert_eq!(pem(&flattened), format!("{key}\n"));
        assert_eq!(pem("not a key"), "not a key");
    }

    #[tokio::test]
    async fn a_missing_or_malformed_fingerprint_is_refused_before_connecting() {
        let credential = Credential::Fields {
            fields: [("password".into(), "s3cret-password".into())].into(),
        };
        let config =
            json!({"host": "127.0.0.1", "port": 9, "username": "u", "fingerprint": "MD5:aa"});
        let cx = Context::new(config, credential);
        let refused = Sftp.call(&cx, "list_dir", json!({})).await;
        assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
        assert_eq!(
            Sftp.call(&cx, "chmod", json!({})).await,
            Err(Error::UnknownTool)
        );
    }

    /// Every tool against a real server: set `CUBEBOT_LIVE_SFTP_HOST`, `_PORT`, `_USER`,
    /// `_FINGERPRINT`, `_ROOT` (a writable folder) and `_PASSWORD` or `_KEY` (a private key file's
    /// path) with any `_PASSPHRASE`, optionally `_ESCAPE` (a symlink in the root leading out of it),
    /// then run `cargo test -p cubebot-connectors live_sftp -- --ignored`.
    #[tokio::test]
    #[ignore = "needs an SSH server"]
    async fn live_sftp() {
        let env = |key: &str| std::env::var(format!("CUBEBOT_LIVE_SFTP_{key}"));
        let mut fields = std::collections::BTreeMap::new();
        if let Ok(path) = env("KEY") {
            // Flattened, as a browser's one-line field sends it.
            let key = std::fs::read_to_string(path).unwrap().replace('\n', "");
            fields.insert("private_key".to_owned(), key);
            if let Ok(passphrase) = env("PASSPHRASE") {
                fields.insert("passphrase".to_owned(), passphrase);
            }
        } else {
            fields.insert("password".to_owned(), env("PASSWORD").unwrap());
        }
        let credential = Credential::Fields { fields };
        let config = json!({"host": env("HOST").unwrap(), "port": env("PORT").unwrap(), "username": env("USER").unwrap(), "fingerprint": env("FINGERPRINT").unwrap(), "root": env("ROOT").unwrap()});
        let cx = Context::new(config.clone(), credential.clone());
        println!("check: {:?}", Sftp.check(&cx).await.unwrap());
        let call = |tool: &'static str, input: Value| {
            let cx = &cx;
            async move {
                let answer = Sftp.call(cx, tool, input).await;
                let shown = format!("{answer:?}");
                println!("{tool}: {}", shown.chars().take(600).collect::<String>());
                answer
            }
        };
        let folder = format!("/cubebot-live-{}", crate::dav::now_nanos());
        call("mkdir", json!({"path": folder})).await.unwrap();
        let file = Attachment::new("a.csv", b"a,b\n1,2\n");
        let path = format!("{folder}/a.csv");
        let written = call("write_file", json!({"path": path, "workspace_file": file}))
            .await
            .unwrap();
        assert_eq!(written["size"], 8);
        let again = call("write_file", json!({"path": path, "workspace_file": file})).await;
        assert!(matches!(again, Err(Error::Rejected(_))));
        call(
            "write_file",
            json!({"path": path, "workspace_file": file, "overwrite": true}),
        )
        .await
        .unwrap();
        let listed = call("list_dir", json!({"path": folder})).await.unwrap();
        assert_eq!(listed["entries"][0]["name"], "a.csv");
        assert_eq!(listed["path"], folder);
        let stat = call("stat", json!({"path": path})).await.unwrap();
        assert_eq!(
            (stat["type"].clone(), stat["size"].clone()),
            (json!("file"), json!(8))
        );
        let read = call("read_file", json!({"path": path})).await.unwrap();
        assert_eq!(read[crate::FILE]["base64"], "YSxiCjEsMgo=");
        let moved = format!("{folder}/b.csv");
        call("rename", json!({"from": path, "to": moved}))
            .await
            .unwrap();
        let gone = call("stat", json!({"path": path})).await;
        assert!(matches!(gone, Err(Error::NotFound(_))), "{gone:?}");
        let escape = call("read_file", json!({"path": "/../../etc/passwd"})).await;
        assert!(matches!(escape, Err(Error::Invalid(_))), "{escape:?}");
        if let Ok(link) = env("ESCAPE") {
            let outside = call("list_dir", json!({"path": format!("/{link}/etc")})).await;
            assert!(matches!(outside, Err(Error::Forbidden(_))), "{outside:?}");
            let made = call("mkdir", json!({"path": format!("/{link}/cubebot-escape")})).await;
            assert!(matches!(made, Err(Error::Forbidden(_))), "{made:?}");
        }
        call("remove", json!({"path": moved})).await.unwrap();
        call("remove", json!({"path": folder})).await.unwrap();
        let mut wrong = config.clone();
        wrong["fingerprint"] = json!("SHA256:uNiVztksCsDhcc0u9e8BujQXVUpKZIDTMczCvj3tD2s");
        let pinned = Sftp.check(&Context::new(wrong, credential)).await;
        assert!(matches!(pinned, Err(Error::Rejected(_))), "{pinned:?}");
        let password = Credential::Fields {
            fields: [("password".into(), "not-the-password".into())].into(),
        };
        assert_eq!(
            Sftp.check(&Context::new(config, password)).await,
            Err(Error::Unauthorized)
        );
    }
}
