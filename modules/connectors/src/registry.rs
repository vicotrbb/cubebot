//! Every compiled-in connector. Adding one is its file and one line here.
use crate::Connector;

/// In the order the console lists them: the most useful first.
static ALL: &[&dyn Connector] = &[
    &crate::google::Google,
    &crate::microsoft::Microsoft,
    &crate::notion::Notion,
    &crate::telegram::Telegram,
];

pub fn all() -> &'static [&'static dyn Connector] {
    ALL
}

pub fn find(id: &str) -> Option<&'static dyn Connector> {
    ALL.iter()
        .copied()
        .find(|connector| connector.manifest().id == id)
}
