//! Pushover notifications (https://pushover.net/api). Both keys travel in the form body. Pushover
//! answers a bad application token or user key with a 400, which here is a refused credential.
use crate::{
    Account, Answer, Auth, Category, Connector, Context, Error, Field, FieldKind, Level, Manifest,
    Risk, Step, ToolSpec,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const API: &str = "https://api.pushover.net/1";

pub struct Pushover;

static MANIFEST: Manifest = Manifest {
    id: "pushover",
    name: "Pushover",
    description: "Send push notifications to your phones and desktops through Pushover.",
    category: Category::Notifications,
    auth: &[Auth::ApiKey {
        fields: &[
            Field {
                key: "token",
                label: "Application API token",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "azGDORePK8gMaC0QOYAMyEEuzJnyUi",
                help: "From the application you create for Cubebot on pushover.net.",
            },
            Field {
                key: "user",
                label: "User key",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "uQiRzpo4DXghDmr9QzzfQu27cmVRsG",
                help: "Your user key from the top of your Pushover dashboard, or a group key.",
            },
        ],
    }],
    config: &[],
    products: &[],
    tools: &[ToolSpec {
        name: "send_notification",
        description: "Send a push notification to the user's devices, or to one device by name.",
        risk: Risk::Send,
        product: None,
        input: r#"{"type":"object","properties":{"message":{"type":"string","description":"Up to 1024 characters."},"title":{"type":"string","description":"Up to 250 characters; the app's name when absent."},"priority":{"type":"integer","minimum":-2,"maximum":2,"description":"-2 no alert, -1 quiet, 0 normal, 1 high, 2 emergency: repeats until acknowledged and needs retry and expire."},"retry":{"type":"integer","minimum":30,"description":"Seconds between emergency repeats, at least 30."},"expire":{"type":"integer","maximum":10800,"description":"Seconds an emergency keeps repeating, at most 10800."},"url":{"type":"string"},"url_title":{"type":"string"},"sound":{"type":"string","description":"A sound's name such as pushover, siren or none."},"device":{"type":"string","description":"Device names separated by commas; all devices when absent."},"html":{"type":"boolean","description":"The message uses Pushover's small HTML subset: b, i, u, font color and a href."}},"required":["message"],"additionalProperties":false}"#,
    }],
    events: &[],
    level: Level::Full,
    guide: &[
        Step {
            title: "Find your user key",
            body: "Sign in at pushover.net. Your user key is at the top of the dashboard; copy it. Install the Pushover app on each device that should get notifications.",
            link: Some("https://pushover.net/"),
            copy: None,
        },
        Step {
            title: "Create an application",
            body: "Choose Create an Application/API Token, name it Cubebot and create it. Copy its API token. Messages count against that application's monthly allowance.",
            link: Some("https://pushover.net/apps/build"),
            copy: Some("Cubebot"),
        },
    ],
    warning: None,
};

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Send {
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    priority: Option<i8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    retry: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expire: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url_title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sound: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    device: Option<String>,
    /// Pushover wants `1`, not `true`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    html: Option<bool>,
}

impl Connector for Pushover {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let valid = post(cx, "users/validate.json", json!({})).await?;
            let devices: Vec<&str> = valid["devices"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let name = match devices.is_empty() {
                true => "Pushover group".into(),
                false => format!("Pushover: {}", devices.join(", ")),
            };
            Ok(Account { name })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "send_notification" => send(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }
}

/// Posts `form` with both keys; Pushover names a bad key `invalid` beside its error.
async fn post(cx: &Context, path: &str, mut form: Value) -> Result<Value, Error> {
    form["token"] = json!(cx.secret("token")?);
    form["user"] = json!(cx.secret("user")?);
    let answer = cx
        .json(cx.post(&format!("{API}/{path}"))?.form(&form))
        .await;
    answer.map_err(|error| match error {
        Error::Rejected(message)
            if ["token is invalid", "user identifier", "user key"]
                .iter()
                .any(|bad| message.contains(bad)) =>
        {
            Error::Unauthorized
        }
        Error::Rejected(message) => Error::Rejected(cx.credential().redact(&message)),
        other => other,
    })
}

async fn send(cx: &Context, input: Value) -> Result<Value, Error> {
    let send: Send = crate::input(input)?;
    if send.message.trim().is_empty() {
        return Err(Error::Invalid("message is empty".into()));
    }
    let mut form = serde_json::to_value(&send).map_err(|_| Error::Invalid("bad input".into()))?;
    if let Some(html) = send.html {
        form["html"] = json!(u8::from(html));
    }
    let sent = post(cx, "messages.json", form).await?;
    Ok(json!({"request": sent["request"], "receipt": sent["receipt"]}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    const TOKEN: &str = "azGDORePK8gMaC0QOYAMyEEuzJnyUi";
    const USER: &str = "uQiRzpo4DXghDmr9QzzfQu27cmVRsG";

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Fields {
            fields: [("token".into(), TOKEN.into()), ("user".into(), USER.into())].into(),
        };
        Context::new(json!({}), credential).with_origin(&fixture.origin())
    }

    fn form(text: &str) -> Vec<(String, String)> {
        let mut pairs: Vec<(String, String)> = text
            .split('&')
            .filter_map(|pair| pair.split_once('='))
            .map(|(key, value)| (key.into(), value.replace('+', " ")))
            .collect();
        pairs.sort();
        pairs
    }

    /// Answer shapes from the API reference.
    #[tokio::test]
    async fn an_emergency_notification_sends_its_repeats_and_answers_the_receipt() {
        let fixture = Fixture::start(vec![Exchange::new(
            "POST",
            "/api.pushover.net/1/messages.json",
            200,
            json!({"status": 1, "request": "647d2300-702c-4b38-8b2f-d56326ae460b", "receipt": "rLqVuqTRh62UzxtmqiaLzQmVcPgiCy"}),
        )])
        .await;
        let input = json!({"message": "Disk full", "title": "nas", "priority": 2, "retry": 60, "expire": 3600, "html": true});
        let sent = Pushover
            .call(&cx(&fixture), "send_notification", input)
            .await
            .unwrap();
        assert_eq!(sent["receipt"], "rLqVuqTRh62UzxtmqiaLzQmVcPgiCy");
        let seen = fixture.finish().await;
        let pairs = |list: &[(&str, &str)]| -> Vec<(String, String)> {
            list.iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect()
        };
        assert_eq!(
            form(&seen[0].text()),
            pairs(&[
                ("expire", "3600"),
                ("html", "1"),
                ("message", "Disk full"),
                ("priority", "2"),
                ("retry", "60"),
                ("title", "nas"),
                ("token", TOKEN),
                ("user", USER),
            ])
        );
    }

    #[tokio::test]
    async fn a_bad_key_is_unauthorized_and_other_refusals_keep_pushovers_words() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "POST",
                "/api.pushover.net/1/users/validate.json",
                400,
                json!({"user": "invalid", "errors": ["user identifier is not a valid user, group, or subscribed user key"], "status": 0, "request": "5042853c-402d-4a18-abcb-168734a801de"}),
            ),
            Exchange::new(
                "POST",
                "/api.pushover.net/1/messages.json",
                400,
                json!({"token": "invalid", "errors": ["application token is invalid"], "status": 0, "request": "a"}),
            ),
            Exchange::new(
                "POST",
                "/api.pushover.net/1/messages.json",
                400,
                json!({"retry": "invalid", "errors": ["retry is invalid"], "status": 0, "request": "b"}),
            ),
        ])
        .await;
        let cx = cx(&fixture);
        assert_eq!(Pushover.check(&cx).await, Err(Error::Unauthorized));
        let input = json!({"message": "hi"});
        let refused = Pushover.call(&cx, "send_notification", input.clone()).await;
        assert_eq!(refused, Err(Error::Unauthorized));
        let refused = Pushover.call(&cx, "send_notification", input).await;
        assert_eq!(refused, Err(Error::Rejected("retry is invalid".into())));
        fixture.finish().await;
        let bad = json!({"message": "hi", "token": "other"});
        let refused = Pushover.call(&cx, "send_notification", bad).await;
        assert!(matches!(refused, Err(Error::Invalid(_))), "{refused:?}");
    }
}
