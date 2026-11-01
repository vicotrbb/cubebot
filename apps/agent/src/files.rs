//! Shared files (ADRs 0059 and 0060) through the orchestrator, which authorizes every call.
//! `files_put` uploads a workspace file as the tool user reads it and `files_get` writes one as
//! the tool user, so neither reaches further than a tool call does. A put's request id is the
//! journaled call's, so a resent call returns the file it already made.
use crate::{
    control::{Error, MemoryClient},
    tools::{Registry, Running},
};
use cubebot_protocol::{
    runtime::{FileShare, ShareKind, SharePermission},
    terminal::Outcome,
};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::io;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    process::{Child, ChildStdout},
};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    parent: Option<String>,
    shared: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Get {
    file: String,
    path: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Put {
    path: String,
    name: String,
    parent: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Share {
    file: String,
    kind: ShareKind,
    name: String,
    permission: SharePermission,
}

/// `workspace` offers `files_get` and `files_put`, which need the tool user.
pub fn definitions(workspace: bool) -> Vec<cubebot_providers::Tool> {
    let tool = |name: &str, description: &str, properties: Value| {
        let required: Vec<&String> = properties
            .as_object()
            .into_iter()
            .flat_map(|o| o.keys())
            .collect();
        cubebot_providers::Tool {
            name: name.into(),
            description: description.into(),
            parameters: json!({"type":"object","properties":properties,"required":required,"additionalProperties":false}),
            strict: true,
            computer: false,
        }
    };
    let text = |description: &str| json!({"type":"string","description":description});
    let folder = json!({"type":["string","null"],"description":"A folder's id from files_list; null for your own top level."});
    let mut tools = vec![
        tool(
            "files_list",
            "List shared files and folders: your own top level, a folder's contents, or with shared true what people and agents shared with you. Shared files outlive your workspace and people download them from the console.",
            json!({"parent":folder,"shared":{"type":"boolean","description":"True to list what is shared with you; parent is then ignored."}}),
        ),
        tool(
            "files_share",
            "Share one of your files or folders, and everything under a folder, with a person by login, a team or an agent by name. Sharing again changes the permission.",
            json!({
                "file":text("The id from files_list or files_put."),
                "kind":{"type":"string","enum":["user","team","agent"]},
                "name":text("The person's login, or the team's or agent's exact name."),
                "permission":{"type":"string","enum":["read","write"],"description":"write also allows uploading into, renaming, moving and deleting."},
            }),
        ),
    ];
    if workspace {
        tools.push(tool(
            "files_put",
            "Upload a workspace file as a new shared file, to hand a deliverable to people or agents. Returns its id; share it with files_share. Re-upload to publish a new version.",
            json!({"path":text("The workspace file."),"name":text("The shared file's name, such as report.pdf."),"parent":folder}),
        ));
        tools.push(tool(
            "files_get",
            "Download a shared file into the workspace, replacing any file at path.",
            json!({"file":text("The id from files_list."),"path":text("Where to write it in the workspace.")}),
        ));
    }
    tools
}

/// `None` when `name` is not a files tool.
pub fn call<'a>(
    client: Option<&'a MemoryClient>,
    name: &str,
    id: Uuid,
    input: &Value,
    tools: Option<&'a Registry>,
) -> Option<Result<Running<'a>, &'static str>> {
    if !matches!(
        name,
        "files_list" | "files_get" | "files_put" | "files_share"
    ) {
        return None;
    }
    let Some(client) = client else {
        return Some(Err("shared files are unavailable"));
    };
    Some(prepare(client, name, id, input, tools))
}

fn prepare<'a>(
    client: &'a MemoryClient,
    name: &str,
    id: Uuid,
    input: &Value,
    tools: Option<&'a Registry>,
) -> Result<Running<'a>, &'static str> {
    let workspace = || tools.ok_or("your tools are disabled, so there is no workspace");
    match name {
        "files_list" => list(client, input),
        "files_share" => share(client, input),
        "files_put" => put(client, id, input, workspace()?),
        _ => get(client, input, workspace()?),
    }
}

fn list<'a>(client: &'a MemoryClient, input: &Value) -> Result<Running<'a>, &'static str> {
    let List { parent, shared } = parse(input)?;
    let parent = folder(parent.as_deref())?;
    Ok(Box::pin(async move {
        let listed = client.files(parent, shared).await;
        reported(listed.map(|listed| listed.map(|entries| json!(entries).to_string())))
    }))
}

fn share<'a>(client: &'a MemoryClient, input: &Value) -> Result<Running<'a>, &'static str> {
    let share: Share = parse(input)?;
    let file = uuid(&share.file)?;
    let share = FileShare {
        kind: share.kind,
        name: share.name,
        permission: share.permission,
    };
    Ok(Box::pin(async move {
        let shared = client.share_file(file, &share).await;
        reported(shared.map(|shared| shared.map(|()| format!("Shared with {}.", share.name))))
    }))
}

fn put<'a>(
    client: &'a MemoryClient,
    id: Uuid,
    input: &Value,
    tools: &'a Registry,
) -> Result<Running<'a>, &'static str> {
    let put: Put = parse(input)?;
    let parent = folder(put.parent.as_deref())?;
    path(&put.path)?;
    Ok(Box::pin(async move {
        reported(upload(client, tools, id, parent, &put).await)
    }))
}

fn get<'a>(
    client: &'a MemoryClient,
    input: &Value,
    tools: &'a Registry,
) -> Result<Running<'a>, &'static str> {
    let get: Get = parse(input)?;
    let file = uuid(&get.file)?;
    path(&get.path)?;
    Ok(Box::pin(async move {
        reported(download(client, tools, file, &get.path).await)
    }))
}

fn parse<T: DeserializeOwned>(input: &Value) -> Result<T, &'static str> {
    T::deserialize(input).map_err(|_| "invalid tool arguments")
}

fn uuid(text: &str) -> Result<Uuid, &'static str> {
    text.parse()
        .map_err(|_| "a file or folder is an id from files_list")
}

fn folder(text: Option<&str>) -> Result<Option<Uuid>, &'static str> {
    text.map(uuid).transpose()
}

fn path(text: &str) -> Result<(), &'static str> {
    if text.is_empty() || text.contains('\0') {
        return Err("path is a workspace file");
    }
    Ok(())
}

fn reported(result: Result<Result<String, String>, Error>) -> Outcome {
    let (code, output) = match result {
        Ok(Ok(output)) => (Some(0), output),
        Ok(Err(reason)) => (Some(1), format!("Refused: {reason}.")),
        Err(_) => (
            None,
            "The orchestrator did not answer; it may or may not have happened.".into(),
        ),
    };
    Outcome::Exited { code, output }
}

async fn upload(
    client: &MemoryClient,
    tools: &Registry,
    id: Uuid,
    parent: Option<Uuid>,
    put: &Put,
) -> Result<Result<String, String>, Error> {
    let unreadable = || Ok(Err(format!("{} cannot be read as the tool user", put.path)));
    let Ok(mut reader) = tools.reader(&put.path) else {
        return unreadable();
    };
    let Some(mut stdout) = reader.stdout.take() else {
        return unreadable();
    };
    // A missing or unreadable file fails here, before anything is sent.
    let Ok(first) = next_chunk(&mut stdout, &mut reader).await else {
        return unreadable();
    };
    let rest = first.is_some().then_some((stdout, reader));
    let chunks = futures_util::stream::unfold((first, rest), |(pending, rest)| async move {
        if let Some(chunk) = pending {
            return Some((Ok(chunk), (None, rest)));
        }
        let (mut stdout, mut reader) = rest?;
        match next_chunk(&mut stdout, &mut reader).await {
            Ok(Some(chunk)) => Some((Ok(chunk), (None, Some((stdout, reader))))),
            Ok(None) => None,
            // The body ends in an error, so the orchestrator stores nothing.
            Err(error) => Some((Err(error), (None, None))),
        }
    });
    let body = reqwest::Body::wrap_stream(chunks);
    let stored = client.put_file(parent, &put.name, id, body).await?;
    Ok(stored.map(|entry| format!("Uploaded: {}", json!(entry))))
}

/// The next chunk the tool user read, `None` once it read the whole file, or an error when it
/// could not.
async fn next_chunk(stdout: &mut ChildStdout, reader: &mut Child) -> io::Result<Option<Vec<u8>>> {
    let mut buffer = vec![0; 64 * 1024];
    let read = stdout.read(&mut buffer).await?;
    if read > 0 {
        buffer.truncate(read);
        return Ok(Some(buffer));
    }
    if reader.wait().await?.success() {
        return Ok(None);
    }
    Err(io::Error::other(
        "the tool user could not read the whole file",
    ))
}

async fn download(
    client: &MemoryClient,
    tools: &Registry,
    file: Uuid,
    path: &str,
) -> Result<Result<String, String>, Error> {
    let mut response = match client.file_content(file).await? {
        Ok(response) => response,
        Err(reason) => return Ok(Err(reason)),
    };
    let unwritable = || Ok(Err(format!("{path} cannot be written as the tool user")));
    let Ok(mut writer) = tools.writer(path) else {
        return unwritable();
    };
    let Some(mut stdin) = writer.stdin.take() else {
        return unwritable();
    };
    let mut size = 0;
    loop {
        // Killed before its input ends, the writer never replaces the file with partial bytes.
        let chunk = match response.chunk().await {
            Ok(Some(chunk)) => chunk,
            Ok(None) => break,
            Err(error) => {
                let _ = writer.kill().await;
                return Err(Error::Unavailable(Some(error.without_url().into())));
            }
        };
        if stdin.write_all(&chunk).await.is_err() {
            let _ = writer.kill().await;
            return unwritable();
        }
        size += chunk.len();
    }
    drop(stdin);
    match writer.wait().await {
        Ok(status) if status.success() => Ok(Ok(format!("Wrote {size} bytes to {path}."))),
        _ => unwritable(),
    }
}
