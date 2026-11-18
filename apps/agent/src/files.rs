//! Transfers run as the tool user, so neither `files_put` nor `files_get` reaches further than a tool call does.
use crate::{
    control::{Error, MemoryClient},
    tools::{Registry, Running, parse},
};
use cubebot_protocol::{
    runtime::{FileShare, ShareKind, SharePermission, TurnFile},
    terminal::Outcome,
};
use cubebot_providers::Message;
use serde::Deserialize;
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
struct Attach {
    path: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Gif {
    url: String,
}

const LARGEST_GIF: usize = 8 * 1024 * 1024;
const MOVING: [(&str, &str); 3] = [
    ("image/gif", "gif"),
    ("image/webp", "webp"),
    ("video/mp4", "mp4"),
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Share {
    file: String,
    kind: ShareKind,
    name: String,
    permission: SharePermission,
}

pub fn definitions(workspace: bool) -> Vec<cubebot_providers::Tool> {
    let tool = cubebot_providers::Tool::strict;
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
            "Share one of your files or folders, and everything under a folder, with a person by login, a group of people or an agent by name, so it appears under Files, Shared with me in their console. Sharing again changes the permission.",
            json!({
                "file":text("The id from files_list or files_put."),
                "kind":{"type":"string","enum":["user","group","agent"]},
                "name":text("The person's login, never their display name: your instructions list logins. For a group or agent, its exact name."),
                "permission":{"type":"string","enum":["read","write"],"description":"write also allows uploading into, renaming, moving and deleting."},
            }),
        ),
    ];
    tools.push(tool(
        "send_gif",
        "Send an animated GIF with your reply; it plays under your message in this conversation or room. Use one when it says something better than words: a reaction, a celebration, a joke that fits the moment and your personality. Pass a direct https link to the GIF, WebP or MP4 file, such as a gif link from a GIPHY search or one you found on the web; never put GIF links in your text, they do not load.",
        json!({"url":text("A direct https link to the GIF, WebP or MP4 file, at most 8 MB.")}),
    ));
    if workspace {
        tools.push(tool(
            "send_attachment",
            "Send a workspace file with your reply, so it appears under your message in this conversation or room: an image shows inline, audio and video get a player, and anything else opens from the message. Use it whenever seeing or hearing the file would serve the person better than describing it. Never write Markdown image links to workspace files; they do not load.",
            json!({"path":text("The workspace file."),"name":text("The name people see, such as chart.png or summary.mp3; the extension decides how it is shown.")}),
        ));
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

const SEEN: [&str; 4] = ["image/png", "image/jpeg", "image/gif", "image/webp"];
const LARGEST_IMAGE: u64 = 5 * 1024 * 1024;

pub async fn images(client: Option<&MemoryClient>, files: &[TurnFile]) -> Vec<Message> {
    let mut images = Vec::new();
    let Some(client) = client else {
        return images;
    };
    for file in files {
        let Some(media_type) = file
            .content_type
            .as_deref()
            .filter(|kind| SEEN.contains(kind))
        else {
            continue;
        };
        if file.size > LARGEST_IMAGE {
            continue;
        }
        let Ok(Ok(response)) = client.file_content(file.file).await else {
            tracing::warn!(file = %file.file, "an attached image could not be read");
            continue;
        };
        let Ok(bytes) = response.bytes().await else {
            continue;
        };
        use base64::Engine;
        images.push(Message::Image {
            media_type: media_type.into(),
            data: base64::engine::general_purpose::STANDARD.encode(bytes),
        });
    }
    images
}

pub fn call<'a>(
    client: Option<&'a MemoryClient>,
    turn: Uuid,
    name: &str,
    id: Uuid,
    input: &Value,
    tools: Option<&'a Registry>,
) -> Option<Result<Running<'a>, &'static str>> {
    if !matches!(
        name,
        "files_list" | "files_get" | "files_put" | "files_share" | "send_attachment" | "send_gif"
    ) {
        return None;
    }
    let Some(client) = client else {
        return Some(Err("shared files are unavailable"));
    };
    let workspace = || tools.ok_or("your tools are disabled, so there is no workspace");
    Some(match name {
        "files_list" => list(client, input),
        "files_share" => share(client, input),
        "send_gif" => gif(client, turn, id, input),
        "files_put" => workspace().and_then(|tools| put(client, id, input, tools)),
        "send_attachment" => workspace().and_then(|tools| attach(client, turn, id, input, tools)),
        _ => workspace().and_then(|tools| get(client, input, tools)),
    })
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
        reported(shared.map(|shared| shared.map(|()| format!("Shared with {}. They find it in the console under Files, Shared with me; there is no link to give them.", share.name))))
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

fn attach<'a>(
    client: &'a MemoryClient,
    turn: Uuid,
    id: Uuid,
    input: &Value,
    tools: &'a Registry,
) -> Result<Running<'a>, &'static str> {
    let attach: Attach = parse(input)?;
    path(&attach.path)?;
    Ok(Box::pin(async move {
        reported(sent(client, tools, turn, id, &attach).await)
    }))
}

async fn sent(
    client: &MemoryClient,
    tools: &Registry,
    turn: Uuid,
    id: Uuid,
    attach: &Attach,
) -> Result<Result<String, String>, Error> {
    let unreadable = || {
        Ok(Err(format!(
            "{} cannot be read as the tool user",
            attach.path
        )))
    };
    let Ok(reader) = tools.reader(&attach.path) else {
        return unreadable();
    };
    let Ok(body) = tool_user_body(reader).await else {
        return unreadable();
    };
    // A same-named earlier file must not refuse the send, so each upload gets a unique name.
    let name = unique_name(&attach.name, id);
    let entry = match client.put_file(None, &name, id, body).await? {
        Ok(entry) => entry,
        Err(reason) => return Ok(Err(reason)),
    };
    let attached = client.attach_file(turn, entry.id).await?;
    Ok(attached.map(|()| {
        format!(
            "Sent {} with your reply. People see it under your message; you do not need to link it.",
            attach.name
        )
    }))
}

fn gif<'a>(
    client: &'a MemoryClient,
    turn: Uuid,
    id: Uuid,
    input: &Value,
) -> Result<Running<'a>, &'static str> {
    let Gif { url } = parse(input)?;
    let url = reqwest::Url::parse(&url)
        .ok()
        .filter(|url| url.scheme() == "https")
        .ok_or("url must be a direct https link to the GIF")?;
    Ok(Box::pin(async move {
        reported(sent_gif(client, turn, id, url).await)
    }))
}

async fn sent_gif(
    client: &MemoryClient,
    turn: Uuid,
    id: Uuid,
    url: reqwest::Url,
) -> Result<Result<String, String>, Error> {
    let (body, extension) = match fetched(url).await {
        Ok(fetched) => fetched,
        Err(reason) => return Ok(Err(reason)),
    };
    let name = unique_name(&format!("gif.{extension}"), id);
    let entry = match client.put_file(None, &name, id, body.into()).await? {
        Ok(entry) => entry,
        Err(reason) => return Ok(Err(reason)),
    };
    let attached = client.attach_file(turn, entry.id).await?;
    Ok(attached.map(|()| {
        "Sent the GIF with your reply. People see it under your message; do not link it.".into()
    }))
}

async fn fetched(url: reqwest::Url) -> Result<(Vec<u8>, &'static str), String> {
    use futures_util::StreamExt;
    let failed = |error: reqwest::Error| format!("the GIF could not be fetched: {error}");
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(failed)?;
    let response = client.get(url).send().await.map_err(failed)?;
    if !response.status().is_success() {
        return Err(format!("the GIF link answered {}", response.status()));
    }
    let extension = moving(response.headers())?;
    let mut body = Vec::new();
    let mut chunks = response.bytes_stream();
    while let Some(chunk) = chunks.next().await {
        body.extend_from_slice(&chunk.map_err(failed)?);
        if body.len() > LARGEST_GIF {
            return Err("the GIF is larger than 8 MB; pick a smaller rendition".into());
        }
    }
    Ok((body, extension))
}

fn moving(headers: &reqwest::header::HeaderMap) -> Result<&'static str, String> {
    let kind = headers
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    MOVING
        .iter()
        .find(|(media, _)| *media == kind)
        .map(|(_, extension)| *extension)
        .ok_or_else(|| format!("the link is {kind:?}, not a GIF, WebP or MP4 file"))
}

fn unique_name(name: &str, id: Uuid) -> String {
    let tag = &id.simple().to_string()[..8];
    match name.rsplit_once('.') {
        Some((stem, extension)) if !stem.is_empty() => format!("{stem}-{tag}.{extension}"),
        _ => format!("{name}-{tag}"),
    }
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
    let Ok(reader) = tools.reader(&put.path) else {
        return unreadable();
    };
    let Ok(body) = tool_user_body(reader).await else {
        return unreadable();
    };
    let stored = client.put_file(parent, &put.name, id, body).await?;
    Ok(stored.map(|entry| format!("Uploaded: {}", json!(entry))))
}

/// A file that fails after the first read ends the body in an error, so nothing is stored.
pub(crate) async fn tool_user_body(mut reader: Child) -> io::Result<reqwest::Body> {
    let mut stdout = reader
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("the reader has no output"))?;
    let first = next_chunk(&mut stdout, &mut reader).await?;
    let rest = first.is_some().then_some((stdout, reader));
    let chunks = futures_util::stream::unfold((first, rest), |(pending, rest)| async move {
        if let Some(chunk) = pending {
            return Some((Ok(chunk), (None, rest)));
        }
        let (mut stdout, mut reader) = rest?;
        match next_chunk(&mut stdout, &mut reader).await {
            Ok(Some(chunk)) => Some((Ok(chunk), (None, Some((stdout, reader))))),
            Ok(None) => None,
            Err(error) => Some((Err(error), (None, None))),
        }
    });
    Ok(reqwest::Body::wrap_stream(chunks))
}

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
    match client.file_content(file).await? {
        Ok(response) => written(response, tools, path).await,
        Err(reason) => Ok(Err(reason)),
    }
}

pub(crate) async fn written(
    mut response: reqwest::Response,
    tools: &Registry,
    path: &str,
) -> Result<Result<String, String>, Error> {
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

#[cfg(test)]
mod tests {
    use super::moving;
    use reqwest::header::{CONTENT_TYPE, HeaderMap, HeaderValue};

    fn typed(kind: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONTENT_TYPE, HeaderValue::from_str(kind).unwrap());
        headers
    }

    #[test]
    fn only_moving_pictures_are_sent_as_gifs() {
        assert_eq!(moving(&typed("image/gif")), Ok("gif"));
        assert_eq!(moving(&typed("Image/WebP; charset=binary")), Ok("webp"));
        assert_eq!(moving(&typed("video/mp4")), Ok("mp4"));
        assert!(moving(&typed("text/html")).is_err());
        assert!(moving(&HeaderMap::new()).is_err());
    }
}
