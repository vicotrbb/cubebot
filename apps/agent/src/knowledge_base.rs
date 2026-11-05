use crate::{
    control::{Error, MemoryClient},
    tools::{Registry, Running, parse},
};
use cubebot_protocol::terminal::Outcome;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Search {
    query: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Read {
    page: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Create {
    kind: String,
    title: String,
    body: String,
    parent: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Edit {
    page: String,
    old_text: String,
    new_text: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Write {
    page: String,
    title: String,
    body: String,
    revision: i64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Move {
    page: String,
    parent: Option<String>,
    after: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Attach {
    page: String,
    path: String,
    name: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Fetch {
    attachment: String,
    path: String,
}

pub fn definitions(workspace: bool) -> Vec<cubebot_providers::Tool> {
    let tool = cubebot_providers::Tool::strict;
    let text = |description: &str| json!({"type":"string","description":description});
    let id = || text("A page's or folder's id from kb_outline or kb_search.");
    let optional = |description: &str| json!({"type":["string","null"],"description":description});
    let mut tools = vec![
        tool(
            "kb_search",
            "Search the knowledge base, the wiki shared by every person and agent here, by words. Search it before you answer questions about how things work here.",
            json!({"query":text("The words to look for.")}),
        ),
        tool(
            "kb_outline",
            "The knowledge base's whole outline: every page and folder with its id, parent and title.",
            json!({}),
        ),
        tool(
            "kb_read",
            "Read a page: its Markdown body, revision and attachments.",
            json!({"page":id()}),
        ),
        tool(
            "kb_create",
            "Create a page or folder. Write what others should be able to find later. Bodies are Markdown: headings, lists, `- [ ]` tasks, tables, code, ![images](url), and ```mermaid fences, which show as diagrams.",
            json!({
                "kind":{"type":"string","enum":["page","folder"]},
                "title":text("Its title."),
                "body":text("The page's Markdown; empty for a folder."),
                "parent":optional("The folder or page to put it in; null for the top level."),
            }),
        ),
        tool(
            "kb_edit",
            "Replace one passage of a page, leaving everything else as it is. The old text must occur exactly once. Prefer this to kb_write: people may be editing the same page.",
            json!({"page":id(),"old_text":text("The exact text to replace."),"new_text":text("What replaces it.")}),
        ),
        tool(
            "kb_write",
            "Replace a page's whole title and body. Refused if the page changed since you read it; read it again and redo your change.",
            json!({"page":id(),"title":text("The title."),"body":text("The whole Markdown body."),"revision":{"type":"integer","description":"The revision kb_read returned."}}),
        ),
        tool(
            "kb_move",
            "Move a page or folder.",
            json!({"page":id(),"parent":optional("The new parent; null for the top level."),"after":optional("The sibling to put it after; null to put it first.")}),
        ),
        tool(
            "kb_delete",
            "Move a page or folder and everything under it to the trash, where people can restore it.",
            json!({"page":id()}),
        ),
    ];
    if workspace {
        tools.push(tool(
            "kb_attach",
            "Attach a workspace file to a page. Returns its URL; put it in the page with kb_edit, as ![name](url) for an image or [name](url) for anything else.",
            json!({"page":id(),"path":text("The workspace file."),"name":text("The attachment's name, such as diagram.png.")}),
        ));
        tools.push(tool(
            "kb_fetch",
            "Download a page's attachment into the workspace, replacing any file at path.",
            json!({"attachment":text("The attachment's id from kb_read."),"path":text("Where to write it in the workspace.")}),
        ));
    }
    tools
}

pub fn call<'a>(
    client: Option<&'a MemoryClient>,
    name: &str,
    id: Uuid,
    input: &Value,
    tools: Option<&'a Registry>,
) -> Option<Result<Running<'a>, &'static str>> {
    if !name.starts_with("kb_") {
        return None;
    }
    let Some(client) = client else {
        return Some(Err("the knowledge base is unavailable"));
    };
    let workspace = || tools.ok_or("your tools are disabled, so there is no workspace");
    Some(match name {
        "kb_search" => search(client, input),
        "kb_outline" => outline(client),
        "kb_read" => read(client, input),
        "kb_create" => create(client, id, input),
        "kb_edit" => edit(client, input),
        "kb_write" => write(client, input),
        "kb_move" => relocate(client, input),
        "kb_delete" => delete(client, input),
        "kb_attach" => workspace().and_then(|tools| attach(client, id, input, tools)),
        "kb_fetch" => workspace().and_then(|tools| fetch(client, input, tools)),
        _ => Err("no such knowledge base tool"),
    })
}

fn search<'a>(client: &'a MemoryClient, input: &Value) -> Result<Running<'a>, &'static str> {
    let Search { query } = parse(input)?;
    Ok(Box::pin(async move {
        reported(client.kb_get("kb/search", &[("q", query.as_str())]).await)
    }))
}

fn outline(client: &MemoryClient) -> Result<Running<'_>, &'static str> {
    Ok(Box::pin(async move {
        reported(client.kb_get("kb/pages", &[]).await)
    }))
}

fn read<'a>(client: &'a MemoryClient, input: &Value) -> Result<Running<'a>, &'static str> {
    let Read { page } = parse(input)?;
    let page = uuid(&page)?;
    Ok(Box::pin(async move {
        reported(client.kb_get(&format!("kb/pages/{page}"), &[]).await)
    }))
}

fn create<'a>(
    client: &'a MemoryClient,
    id: Uuid,
    input: &Value,
) -> Result<Running<'a>, &'static str> {
    let create: Create = parse(input)?;
    let parent = optional(create.parent.as_deref())?;
    // The journaled call's id, so a resent call finds the page it made.
    let body = json!({"request_id":id,"parent":parent,"kind":create.kind,"title":create.title,"body":create.body});
    Ok(Box::pin(async move {
        let sent = client
            .kb_send(reqwest::Method::POST, "kb/pages", &body)
            .await;
        reported(sent.map(|reply| reply.map(summarized)))
    }))
}

fn edit<'a>(client: &'a MemoryClient, input: &Value) -> Result<Running<'a>, &'static str> {
    let edit: Edit = parse(input)?;
    let page = uuid(&edit.page)?;
    let body = json!({"replace":{"old_text":edit.old_text,"new_text":edit.new_text}});
    Ok(Box::pin(async move {
        changed(
            client,
            reqwest::Method::PATCH,
            format!("kb/pages/{page}"),
            body,
        )
        .await
    }))
}

fn write<'a>(client: &'a MemoryClient, input: &Value) -> Result<Running<'a>, &'static str> {
    let write: Write = parse(input)?;
    let page = uuid(&write.page)?;
    let body = json!({"expected_revision":write.revision,"title":write.title,"body":write.body});
    Ok(Box::pin(async move {
        changed(
            client,
            reqwest::Method::PATCH,
            format!("kb/pages/{page}"),
            body,
        )
        .await
    }))
}

fn relocate<'a>(client: &'a MemoryClient, input: &Value) -> Result<Running<'a>, &'static str> {
    let target: Move = parse(input)?;
    let page = uuid(&target.page)?;
    let body = json!({"parent":optional(target.parent.as_deref())?,"after":optional(target.after.as_deref())?});
    Ok(Box::pin(async move {
        changed(
            client,
            reqwest::Method::POST,
            format!("kb/pages/{page}/place"),
            body,
        )
        .await
    }))
}

fn delete<'a>(client: &'a MemoryClient, input: &Value) -> Result<Running<'a>, &'static str> {
    let Read { page } = parse(input)?;
    let page = uuid(&page)?;
    Ok(Box::pin(async move {
        changed(
            client,
            reqwest::Method::DELETE,
            format!("kb/pages/{page}"),
            Value::Null,
        )
        .await
    }))
}

fn attach<'a>(
    client: &'a MemoryClient,
    id: Uuid,
    input: &Value,
    tools: &'a Registry,
) -> Result<Running<'a>, &'static str> {
    let attach: Attach = parse(input)?;
    let page = uuid(&attach.page)?;
    Ok(Box::pin(async move {
        let Ok(reader) = tools.reader(&attach.path) else {
            return unreadable(&attach.path);
        };
        let Ok(body) = crate::files::tool_user_body(reader).await else {
            return unreadable(&attach.path);
        };
        let attached = client.kb_attach(page, &attach.name, id, body).await;
        reported(attached.map(|reply| {
            reply.map(|text| {
                let id = serde_json::from_str::<Value>(&text)
                    .ok()
                    .and_then(|entry| entry["id"].as_str().map(str::to_owned))
                    .unwrap_or_default();
                format!("Attached. Its URL is /api/kb/attachments/{id}")
            })
        }))
    }))
}

fn fetch<'a>(
    client: &'a MemoryClient,
    input: &Value,
    tools: &'a Registry,
) -> Result<Running<'a>, &'static str> {
    let fetch: Fetch = parse(input)?;
    let attachment = uuid(&fetch.attachment)?;
    Ok(Box::pin(async move {
        let response = match client.kb_attachment(attachment).await {
            Ok(Ok(response)) => response,
            Ok(Err(reason)) => return reported(Ok(Err(reason))),
            Err(error) => return reported(Err(error)),
        };
        reported(crate::files::written(response, tools, &fetch.path).await)
    }))
}

async fn changed(
    client: &MemoryClient,
    method: reqwest::Method,
    path: String,
    body: Value,
) -> Outcome {
    let sent = client.kb_send(method, &path, &body).await;
    reported(sent.map(|reply| reply.map(summarized)))
}

fn summarized(reply: String) -> String {
    let Ok(mut page) = serde_json::from_str::<Value>(&reply) else {
        return "Done.".into();
    };
    if let Some(page) = page.as_object_mut() {
        page.remove("body");
    }
    format!("Done: {page}")
}

fn unreadable(path: &str) -> Outcome {
    reported(Ok(Err(format!("{path} cannot be read as the tool user"))))
}

fn uuid(text: &str) -> Result<Uuid, &'static str> {
    text.parse()
        .map_err(|_| "a page is an id from kb_outline or kb_search")
}

fn optional(text: Option<&str>) -> Result<Option<Uuid>, &'static str> {
    text.map(uuid).transpose()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_that_move_bytes_need_the_workspace() {
        let names = |workspace| -> Vec<String> {
            definitions(workspace)
                .into_iter()
                .map(|tool| tool.name)
                .collect()
        };
        assert!(names(true).contains(&"kb_attach".to_owned()));
        assert!(!names(false).contains(&"kb_attach".to_owned()));
        assert!(names(false).contains(&"kb_edit".to_owned()));
    }

    #[test]
    fn a_write_reply_leaves_out_the_body() {
        let reply = json!({"id":"x","revision":3,"body":"long"}).to_string();
        let summary = summarized(reply);
        assert!(summary.contains("\"revision\":3") && !summary.contains("long"));
    }

    #[test]
    fn every_offered_tool_is_handled_without_a_client() {
        for tool in definitions(true) {
            let result = call(None, &tool.name, Uuid::nil(), &json!({}), None).unwrap();
            assert!(result.is_err());
        }
        assert!(call(None, "files_list", Uuid::nil(), &json!({}), None).is_none());
    }
}
