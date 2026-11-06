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
    &crate::dropbox::Dropbox,
    &crate::cal_com::CalCom,
    &crate::zoom::Zoom,
    &crate::stripe::Stripe,
];

pub fn all() -> &'static [&'static dyn Connector] {
    ALL
}

pub fn find(id: &str) -> Option<&'static dyn Connector> {
    ALL.iter()
        .copied()
        .find(|connector| connector.manifest().id == id)
}
