//! Installed skills (ADR 0055). The system prompt names each installed skill; `load_skill` fetches
//! the installed revision on demand, writes its files into the workspace as the tool user and
//! returns its instructions, so its scripts run through the ordinary tools under the same limits
//! as any other command. The library and installations belong to the orchestrator.
use crate::{
    control::MemoryClient,
    tools::{Registry, Running},
};
use cubebot_protocol::{runtime::SkillPackage, terminal::Outcome};
use serde::Deserialize;
use serde_json::{Value, json};

/// Workspace directory packages are written to, one directory per skill revision.
const SKILLS: &str = ".cubebot-skills";

pub struct Skills {
    client: Option<MemoryClient>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Load {
    name: String,
}

pub fn definitions() -> Vec<cubebot_providers::Tool> {
    vec![cubebot_providers::Tool {
        name: "load_skill".into(),
        description: "Load one of your installed skills by the name your instructions list, before work it covers. Returns its instructions; its supporting files are written to the workspace, and you run its scripts with your ordinary tools.".into(),
        parameters: json!({"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false}),
        strict: true,
    }]
}

impl Skills {
    pub fn new(client: Option<MemoryClient>) -> Self {
        Self { client }
    }

    /// `None` when `name` is not a skill tool.
    pub fn call<'a>(
        &'a self,
        name: &str,
        input: &Value,
        tools: Option<&'a Registry>,
    ) -> Option<Result<Running<'a>, &'static str>> {
        if name != "load_skill" {
            return None;
        }
        let load = Load::deserialize(input)
            .ok()
            .filter(|load| named(&load.name))
            .ok_or("name must be an installed skill's name");
        Some(load.map(|Load { name }| -> Running<'a> { Box::pin(self.load(name, tools)) }))
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
        let skills = Skills::new(None);
        assert!(skills.call("bash", &json!({}), None).is_none());
        for name in ["../memories", "Disk", "a--b", ""] {
            assert!(
                skills
                    .call("load_skill", &json!({"name":name}), None)
                    .unwrap()
                    .is_err()
            );
        }
        let Outcome::Exited { code: None, output } = skills
            .call("load_skill", &json!({"name":"disk-triage"}), None)
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
