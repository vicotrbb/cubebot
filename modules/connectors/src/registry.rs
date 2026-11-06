//! Every compiled-in connector. Adding one is its file and one line here.
use crate::Connector;

static ALL: &[&dyn Connector] = &[&crate::notion::Notion];

pub fn all() -> &'static [&'static dyn Connector] {
    ALL
}

pub fn find(id: &str) -> Option<&'static dyn Connector> {
    ALL.iter()
        .copied()
        .find(|connector| connector.manifest().id == id)
}
