use super::Collaboration;
use crate::{connections::Connections, tools::Running};
use cubebot_protocol::{apps::valid_folder, runtime::RoutineCall, terminal::Outcome};
use serde_json::{Value, json};
use std::path::Path;
use uuid::Uuid;

const NAMES: [&str; 5] = [
    "app_register",
    "app_update",
    "app_state",
    "app_remove",
    "app_list",
];

const TEST: &str = " The result names the URL at http://127.0.0.1:7400/{slug}/ where you open it in your own browser to test it.";

pub fn definitions() -> Vec<cubebot_providers::Tool> {
    let tool = |name: &str, description: &str, parameters: Value| {
        cubebot_providers::Tool::strict(name, description, parameters)
    };
    let text = |description: &str| json!({"type":"string","description":description});
    let maybe = |description: &str| json!({"type":["string","null"],"description":description});
    let port = json!({"type":["integer","null"],"description":"The port your backend listens on, 1024 to 65535 and not 7400; null for a static app."});
    let tools = |description: &str| json!({"type":"array","items":{"type":"string"},"description":description});
    vec![
        tool(
            "app_register",
            &format!(
                "Register a small web app you built in your workspace so people can open it in the console. Load the build-app skill first. With a command and a port it is a backend app the platform starts and restarts; with neither it serves the folder's static files.{TEST}"
            ),
            json!({
                "slug":text("Lowercase letters, digits and hyphens, 2 to 40, unique on the platform; it is the app's address."),
                "name":text("The name people see, up to 60 characters."),
                "description":text("What it is for, up to 300 characters."),
                "icon":text("One emoji."),
                "folder":text("A relative folder inside your workspace that already exists, such as apps/budget."),
                "command":maybe("The shell command that starts the backend in the folder, listening on the port; null for a static app."),
                "port":port.clone(),
                "tools":tools("Every connection tool the app calls, named connection__tool; the app can call nothing else."),
            }),
        ),
        tool(
            "app_update",
            &format!(
                "Change an app you registered. Null keeps a field as it is. Command and port cannot be cleared: to turn a backend app into a static one, remove it and register it again.{TEST}"
            ),
            json!({
                "slug":text("The app's slug."),
                "name":maybe("The new name, or null."),
                "description":maybe("The new description, or null."),
                "icon":maybe("The new emoji, or null."),
                "folder":maybe("The new workspace folder, or null."),
                "command":maybe("The new start command, or null to keep it."),
                "port":port,
                "tools":{"type":["array","null"],"items":{"type":"string"},"description":"The full new list of connection tools the app may call, or null to keep it."},
            }),
        ),
        tool(
            "app_state",
            &format!(
                "Start, stop or mark one of your apps as being updated. Set updating before you change a running app, so people see that it is being worked on, and running afterwards. Running and stopped also clear updating.{TEST}"
            ),
            json!({
                "slug":text("The app's slug."),
                "state":{"type":"string","enum":["running","stopped","updating"],"description":"The state to set."},
            }),
        ),
        tool(
            "app_remove",
            "Remove one of your apps from the console. Its folder stays in your workspace.",
            json!({"slug":text("The app's slug.")}),
        ),
        tool(
            "app_list",
            "List your apps with their folder, command, port, tools and whether each should run.",
            json!({}),
        ),
    ]
}

fn refusal(workspace: &Path, input: &Value, connections: &Connections) -> Option<String> {
    if let Some(folder) = input["folder"].as_str() {
        if !valid_folder(folder) {
            return Some("Refused: folder is a relative path inside the workspace with no empty, . or .. part, no part starting with a dot and no backslash.".into());
        }
        let inside = workspace
            .join(folder)
            .canonicalize()
            .ok()
            .zip(workspace.canonicalize().ok())
            .is_some_and(|(path, root)| path.is_dir() && path != root && path.starts_with(root));
        if !inside {
            return Some(format!(
                "Refused: the folder {folder} does not exist in your workspace; create it and put the app in it first."
            ));
        }
    }
    let missing = input["tools"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .find(|tool| !connections.offers(tool))?;
    Some(format!(
        "Refused: {missing} is not offered by a connection you may use; check the name with find_tools."
    ))
}

impl Collaboration {
    pub(super) fn app(
        &self,
        name: &str,
        id: Uuid,
        input: &Value,
    ) -> Option<Result<Running<'_>, &'static str>> {
        if !NAMES.contains(&name) {
            return None;
        }
        let call = RoutineCall {
            call: id,
            tool: name.into(),
            input: input.clone(),
        };
        let checked = matches!(name, "app_register" | "app_update");
        Some(Ok(Box::pin(async move {
            if checked && let Some(reason) = self.precheck(&call.input).await {
                return Outcome::Exited {
                    code: Some(1),
                    output: reason,
                };
            }
            self.forward(call).await
        })))
    }

    async fn precheck(&self, input: &Value) -> Option<String> {
        let client = self.client.as_ref()?;
        let workspace = std::env::var_os("CUBEBOT_WORKSPACE_DIRECTORY")?;
        let granted = client.connections().await.ok()?;
        let connections = Connections::new(Some(client.clone()), &granted);
        refusal(Path::new(&workspace), input, &connections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workspace() -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("apps-{}", Uuid::new_v4()));
        std::fs::create_dir_all(path.join("budget")).unwrap();
        path
    }

    #[test]
    fn register_refuses_a_missing_folder() {
        let root = workspace();
        let connections = Connections::new(None, &[]);
        let missing = refusal(&root, &json!({"folder": "nope", "tools": []}), &connections);
        assert!(missing.unwrap().contains("does not exist"));
        assert!(
            refusal(
                &root,
                &json!({"folder": "budget", "tools": []}),
                &connections
            )
            .is_none()
        );
        for bad in [
            ".",
            "..",
            "../x",
            "a/../budget",
            ".hidden",
            "budget/.git",
            "a\\b",
            "/etc",
        ] {
            let refused = refusal(&root, &json!({"folder": bad}), &connections);
            assert!(refused.unwrap().contains("relative path"), "{bad}");
        }
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn register_refuses_an_undeclared_connection_tool() {
        let root = workspace();
        let connections = Connections::new(None, &[]);
        let input = json!({"folder": "budget", "tools": ["whoop__recovery"]});
        let refused = refusal(&root, &input, &connections);
        assert!(refused.unwrap().contains("whoop__recovery"));
        let _ = std::fs::remove_dir_all(root);
    }
}
