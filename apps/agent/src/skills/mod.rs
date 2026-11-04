//! Installed skills (docs/adr/memory.md#reusable-skills-created-by-humans-agents-and-dreaming). The system prompt names each installed skill; `load_skill` fetches
//! the installed revision on demand, writes its files into the workspace as the tool user and
//! returns its instructions, so its scripts run through the ordinary tools under the same limits
//! as any other command. The library and installations belong to the orchestrator.
use crate::{
    control::MemoryClient,
    tools::{Registry, Running},
};
use cubebot_protocol::{
    runtime::{RoutineCall, SkillPackage},
    terminal::Outcome,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

/// Workspace directory packages are written to, one directory per skill revision.
const SKILLS: &str = ".cubebot-skills";

const TOOLS: [&str; 4] = [
    "skill_list",
    "skill_create",
    "skill_update",
    "skill_install",
];

pub struct Skills {
    client: Option<MemoryClient>,
    turn: Uuid,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Load {
    name: String,
}

pub fn definitions() -> Vec<cubebot_providers::Tool> {
    let tool = cubebot_providers::Tool::strict;
    let text = |description: &str| json!({"type":"string","description":description});
    let name = text("Lowercase words joined by hyphens, such as disk-triage.");
    let description = text(
        "One or two sentences saying what it does and exactly when to load it. It is all you will see of the skill until you load it, so name the situations.",
    );
    let instructions = text(
        "The steps as Markdown, without front matter: what to do, in what order, and how to check it worked. Generalize: no credentials, personal details or one-off specifics.",
    );
    let files = json!({"type":"array","description":"Supporting files, usually none.","items":{"type":"object","properties":{
        "path":text("A relative path under references/, scripts/ or assets/."),
        "content":text("The file's text."),
    },"required":["path","content"],"additionalProperties":false}});
    vec![
        tool(
            "load_skill",
            "Load one of your installed skills by the name your instructions list, before work it covers. Returns its instructions; its supporting files are written to the workspace, and you run its scripts with your ordinary tools.",
            json!({"name":{"type":"string"}}),
        ),
        tool(
            "skill_list",
            "List every skill in this installation's shared library, with its current revision and the revision you have installed, if any.",
            json!({}),
        ),
        tool(
            "skill_create",
            "Publish a new skill to the shared library and install it for yourself. This is the only way to create a skill: a file or knowledge base page called a skill is not one. Use it when someone asks you to create or save a skill, or after a multi-step procedure worked and will clearly recur. Do not write one for a one-off task, or when an existing skill could be improved instead.",
            json!({"name":name,"description":description,"instructions":instructions,"files":files}),
        ),
        tool(
            "skill_update",
            "Publish a new revision of a library skill and move your installation to it, when you found it wrong or missing a step. Load it first and send the whole new version; other agents keep their revision until someone updates them.",
            json!({"name":name,"expected_revision":{"type":"integer","description":"The current revision from skill_list."},"description":description,"instructions":instructions,"files":files}),
        ),
        tool(
            "skill_install",
            "Install a library skill for yourself at its current revision, or move your installation to it, so you can load it.",
            json!({"name":name}),
        ),
    ]
}

impl Skills {
    pub fn new(client: Option<MemoryClient>, turn: Uuid) -> Self {
        Self { client, turn }
    }

    /// `None` when `name` is neither a skill tool nor a shared files tool, which also moves
    /// orchestrator content through the workspace (see [`crate::files`]).
    pub fn call<'a>(
        &'a self,
        name: &str,
        id: Uuid,
        input: &Value,
        tools: Option<&'a Registry>,
    ) -> Option<Result<Running<'a>, &'static str>> {
        if TOOLS.contains(&name) {
            return Some(Ok(Box::pin(self.manage(
                name.to_owned(),
                id,
                input.clone(),
            ))));
        }
        if name != "load_skill" {
            return crate::files::call(self.client.as_ref(), self.turn, name, id, input, tools)
                .or_else(|| {
                    crate::knowledge_base::call(self.client.as_ref(), name, id, input, tools)
                });
        }
        let load = Load::deserialize(input)
            .ok()
            .filter(|load| named(&load.name))
            .ok_or("name must be an installed skill's name");
        Some(load.map(|Load { name }| -> Running<'a> { Box::pin(self.load(name, tools)) }))
    }

    /// The orchestrator validates each call and answers with the text the model reads.
    async fn manage(&self, tool: String, id: Uuid, input: Value) -> Outcome {
        let call = RoutineCall {
            call: id,
            tool,
            input,
        };
        let sent = match &self.client {
            Some(client) => client.skill_tool(self.turn, &call).await,
            None => Err(crate::control::Error::Unavailable(None)),
        };
        let (code, output) = match sent {
            Ok(Ok(output)) => (Some(0), output),
            Ok(Err(reason)) => (Some(1), format!("Refused: {reason}.")),
            Err(_) => (
                None,
                "The orchestrator did not answer; it may or may not have happened.".into(),
            ),
        };
        Outcome::Exited { code, output }
    }

    async fn load(&self, name: String, tools: Option<&Registry>) -> Outcome {
        let found = match &self.client {
            Some(client) => client.skill(&name).await.ok(),
            None => None,
        };
        match found {
            Some(Some(package)) => Outcome::Exited {
                code: Some(0),
                output: materialize(&package, tools).await,
            },
            Some(None) => Outcome::Exited {
                code: Some(1),
                output: format!("No installed skill is called {name}."),
            },
            None => Outcome::Exited {
                code: None,
                output: "Skills are unavailable right now.".into(),
            },
        }
    }
}

/// The package's instructions, after writing all of its files under one revision's directory.
async fn materialize(package: &SkillPackage, tools: Option<&Registry>) -> String {
    let entry = package.files.get("SKILL.md").cloned().unwrap_or_default();
    let others: Vec<&str> = package
        .files
        .keys()
        .map(String::as_str)
        .filter(|path| *path != "SKILL.md")
        .collect();
    if others.is_empty() {
        return entry;
    }
    let others = others.join(", ");
    let Some(tools) = tools else {
        return format!(
            "{entry}\n\nThis skill has supporting files ({others}), but your tools are disabled, so they are unavailable."
        );
    };
    let directory = format!("{SKILLS}/{}/{}", package.name, package.revision);
    for (path, content) in &package.files {
        if !tools.write(&format!("{directory}/{path}"), content).await {
            return format!(
                "{entry}\n\nWriting this skill's files to {directory} failed, so its supporting files ({others}) are unavailable."
            );
        }
    }
    format!(
        "{entry}\n\nThis skill's files are in the workspace under {directory}/, where its relative paths start: {others}. Run its scripts with your tools, for example bash {directory}/scripts/<name>."
    )
}

/// Skill names are lowercase words joined by hyphens, so one never changes the request path.
fn named(name: &str) -> bool {
    name.len() <= 64
        && name.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn loading_refuses_bad_names_and_reports_unavailable_without_a_client() {
        let skills = Skills::new(None, Uuid::nil());
        assert!(skills.call("bash", Uuid::nil(), &json!({}), None).is_none());
        for name in ["../memories", "Disk", "a--b", ""] {
            assert!(
                skills
                    .call("load_skill", Uuid::nil(), &json!({"name":name}), None)
                    .unwrap()
                    .is_err()
            );
        }
        let Outcome::Exited { code: None, output } = skills
            .call(
                "load_skill",
                Uuid::nil(),
                &json!({"name":"disk-triage"}),
                None,
            )
            .unwrap()
            .unwrap()
            .await
        else {
            panic!("loading without a client must report unavailable");
        };
        assert!(output.contains("unavailable"));
    }

    #[tokio::test]
    async fn without_tools_only_the_instructions_are_returned() {
        let package = SkillPackage {
            name: "disk-triage".into(),
            revision: 2,
            files: [
                ("SKILL.md".into(), "Run the script.".into()),
                ("scripts/usage.sh".into(), "du -sh *".into()),
            ]
            .into(),
        };
        let shown = materialize(&package, None).await;
        assert!(shown.starts_with("Run the script.") && shown.contains("scripts/usage.sh"));
        assert!(shown.contains("tools are disabled"), "{shown}");
    }
}
