//! Every compiled-in connector. Adding one is its file and one line here.
use crate::Connector;

/// In the order the console lists them: the most useful first.
static ALL: &[&dyn Connector] = &[
    &crate::email::Email,
    &crate::google::Google,
    &crate::microsoft::Microsoft,
    &crate::dav::Dav,
    &crate::github::Github,
    &crate::notion::Notion,
    &crate::slack::Slack,
    &crate::telegram::Telegram,
    &crate::http_api::HttpApi,
    &crate::linear::Linear,
    &crate::atlassian::Atlassian,
    &crate::asana::Asana,
    &crate::todoist::Todoist,
    &crate::trello::Trello,
    &crate::airtable::Airtable,
    &crate::hubspot::Hubspot,
    &crate::discord::Discord,
    &crate::readwise::Readwise,
    &crate::rss::Rss,
    &crate::calendly::Calendly,
];

pub fn all() -> &'static [&'static dyn Connector] {
    ALL
}

pub fn find(id: &str) -> Option<&'static dyn Connector> {
    ALL.iter()
        .copied()
        .find(|connector| connector.manifest().id == id)
}
