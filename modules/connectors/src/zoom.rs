//! Zoom meetings and cloud recordings through the person's own Server-to-Server OAuth app
//! (https://developers.zoom.us/docs/internal-apps/s2s-oauth/, https://developers.zoom.us/docs/internal-apps/create/,
//! https://developers.zoom.us/docs/api/meetings/, https://developers.zoom.us/docs/internal-apps/oauth-scopes-granular/).
//! The orchestrator trades the app's credentials for hour-long tokens; there is no refresh token.
//! A Server-to-Server app has no signed-in user, so the connection names the user to act for.
use crate::{
    Account, Answer, Auth, Category, ClientCredentials, Connector, Context, Error, Event,
    EventSpec, Field, FieldKind, LARGEST, Level, Manifest, Poll, Risk, Step, ToolSpec,
};
use reqwest::{RequestBuilder, Response, Url, header};
use serde::Deserialize;
use serde_json::{Value, json};

const API: &str = "https://api.zoom.us/v2";
/// The most characters of a transcript or agenda the model gets.
const LONGEST: usize = 50_000;
const REDIRECTS: usize = 3;
/// Meetings with recordings each poll looks at, and how many reported ones the cursor keeps.
const POLLED: u16 = 30;
const REMEMBERED: usize = 200;
const DAY: u64 = 24 * 60 * 60;

pub struct Zoom;

static MANIFEST: Manifest = Manifest {
    id: "zoom",
    name: "Zoom",
    description: "Read Zoom meetings and cloud recordings and transcripts, and hear when a recording is ready.",
    category: Category::MailCalendar,
    auth: &[Auth::ClientCredentials(ClientCredentials {
        token: "https://zoom.us/oauth/token",
        grant: "account_credentials",
        params: &[("account_id", "{account_id}")],
        fields: &[
            Field {
                key: "account_id",
                label: "Account ID",
                kind: FieldKind::Text,
                required: true,
                placeholder: "",
                help: "From the app's App credentials page.",
            },
            Field {
                key: "client_id",
                label: "Client ID",
                kind: FieldKind::Text,
                required: true,
                placeholder: "",
                help: "From the app's App credentials page.",
            },
            Field {
                key: "client_secret",
                label: "Client secret",
                kind: FieldKind::Secret,
                required: true,
                placeholder: "",
                help: "From the app's App credentials page.",
            },
        ],
    })],
    config: &[Field {
        key: "user",
        label: "Zoom user",
        kind: FieldKind::Text,
        required: true,
        placeholder: "you@example.com",
        help: "The email address of the Zoom user whose meetings and recordings agents see.",
    }],
    products: &[],
    tools: &[
        ToolSpec {
            name: "list_meetings",
            description: "The user's upcoming or previous meetings; pass next for more.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"when":{"type":"string","enum":["upcoming","previous"],"description":"upcoming when absent."},"limit":{"type":"integer","minimum":1,"maximum":300},"next":{"type":"string","description":"The next token of a previous answer."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "read_meeting",
            description: "A meeting's topic, agenda, time, join link and settings summary.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"meeting":{"type":"string","description":"The meeting's numeric id."}},"required":["meeting"],"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "list_recordings",
            description: "Cloud recordings of the user's meetings in a date range of up to a month, with their files.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"from":{"type":"string","description":"yyyy-mm-dd in UTC; a week ago when absent."},"to":{"type":"string","description":"yyyy-mm-dd in UTC; today when absent."},"limit":{"type":"integer","minimum":1,"maximum":300},"next":{"type":"string","description":"The next token of a previous answer."}},"additionalProperties":false}"#,
        },
        ToolSpec {
            name: "download_recording",
            description: "A recording file: transcripts and chat as text, video and audio saved to the workspace.",
            risk: Risk::Read,
            product: None,
            input: r#"{"type":"object","properties":{"meeting":{"type":"string","description":"The meeting's uuid (or numeric id for its latest instance) from list_recordings."},"file":{"type":"string","description":"The file's id from list_recordings."}},"required":["meeting","file"],"additionalProperties":false}"#,
        },
    ],
    events: &[EventSpec {
        kind: "recording.completed",
        description: "A meeting's cloud recording finished processing.",
        product: None,
    }],
    level: Level::Read,
    guide: &[
        Step {
            title: "Create a Server-to-Server OAuth app",
            body: "Sign in to the Zoom App Marketplace, open Developers (lower left) to reach Created apps, choose the + next to Developer and Build an app, pick Server-to-Server OAuth app and Create. If that type is missing, an account admin must give your role the Server-to-Server OAuth app permission under Role Management.",
            link: Some("https://marketplace.zoom.us/"),
            copy: None,
        },
        Step {
            title: "Fill in the Information page",
            body: "Add a short description, a company name, and a developer name and email; activation needs them.",
            link: None,
            copy: None,
        },
        Step {
            title: "Add the scopes",
            body: "On the Scopes page choose Add Scopes and add these, then save.",
            link: None,
            copy: Some(
                "user:read:user:admin meeting:read:list_meetings:admin meeting:read:meeting:admin cloud_recording:read:list_user_recordings:admin cloud_recording:read:list_recording_files:admin",
            ),
        },
        Step {
            title: "Activate the app and copy its credentials",
            body: "On the Activation page choose Activate your app. Then copy the Account ID, Client ID and Client secret from App credentials into the form below, with the email address of the Zoom user to act for.",
            link: None,
            copy: None,
        },
    ],
    warning: Some(
        "The app acts for the whole Zoom account, so its scopes reach every user's meetings, not only the one named here. Deactivating the app stops the connection.",
    ),
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Meetings {
    when: Option<String>,
    limit: Option<u16>,
    next: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Meeting {
    meeting: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Recordings {
    from: Option<String>,
    to: Option<String>,
    limit: Option<u16>,
    next: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Download {
    meeting: String,
    file: String,
}

impl Connector for Zoom {
    fn manifest(&self) -> &'static Manifest {
        &MANIFEST
    }

    fn check<'a>(&'a self, cx: &'a Context) -> Answer<'a, Account> {
        Box::pin(async move {
            let me = cx
                .json(cx.get(&format!("{API}/users/{}", user(cx)?))?)
                .await?;
            let name = me["email"].as_str().unwrap_or("Zoom user");
            Ok(Account { name: name.into() })
        })
    }

    fn call<'a>(&'a self, cx: &'a Context, tool: &'a str, input: Value) -> Answer<'a, Value> {
        Box::pin(async move {
            match tool {
                "list_meetings" => meetings(cx, input).await,
                "read_meeting" => meeting(cx, input).await,
                "list_recordings" => recordings(cx, input).await,
                "download_recording" => download(cx, input).await,
                _ => Err(Error::UnknownTool),
            }
        })
    }

    fn poll<'a>(&'a self, cx: &'a Context, cursor: Option<&'a str>) -> Answer<'a, Poll> {
        Box::pin(poll(cx, cursor))
    }
}

/// The configured user, as one path segment.
fn user(cx: &Context) -> Result<String, Error> {
    let user = cx
        .setting("user")
        .ok_or_else(|| Error::Invalid("the connection names no Zoom user".into()))?;
    Ok(segment(user.trim()))
}

/// Percent-encodes everything but unreserved characters.
fn segment(text: &str) -> String {
    text.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(b).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// A meeting's numeric id, or an instance's uuid; Zoom wants a uuid that starts with `/` or holds
/// `//` encoded twice.
fn meeting_path(text: &str) -> Result<String, Error> {
    let text = text.trim();
    let digits: String = text.chars().filter(|c| *c != ' ').collect();
    if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(digits);
    }
    let uuid = !text.is_empty()
        && text.len() <= 64
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b));
    match uuid {
        true if text.starts_with('/') || text.contains("//") => Ok(segment(&segment(text))),
        true => Ok(segment(text)),
        false => Err(Error::Invalid(format!(
            "{text:?} is not a Zoom meeting id or uuid"
        ))),
    }
}

async fn meetings(cx: &Context, input: Value) -> Result<Value, Error> {
    let wanted: Meetings = crate::input(input)?;
    let kind = match wanted.when.as_deref() {
        None | Some("upcoming") => "upcoming",
        Some("previous") => "previous_meetings",
        Some(other) => {
            return Err(Error::Invalid(format!(
                "{other:?} is not upcoming or previous"
            )));
        }
    };
    let size = wanted.limit.unwrap_or(30).clamp(1, 300).to_string();
    let mut query = vec![("type", kind), ("page_size", size.as_str())];
    if let Some(next) = &wanted.next {
        query.push(("next_page_token", next));
    }
    let path = format!("{API}/users/{}/meetings", user(cx)?);
    let found = cx.json(cx.get(&path)?.query(&query)).await?;
    let shown: Vec<Value> = found["meetings"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|item| {
            json!({
                "id": item["id"],
                "uuid": item["uuid"],
                "topic": item["topic"],
                "start": item["start_time"],
                "minutes": item["duration"],
                "timezone": item["timezone"],
                "join_url": item["join_url"],
            })
        })
        .collect();
    Ok(json!({"meetings": shown, "next": next(&found)}))
}

fn next(found: &Value) -> Value {
    match found["next_page_token"].as_str() {
        Some(token) if !token.is_empty() => json!(token),
        _ => Value::Null,
    }
}

async fn meeting(cx: &Context, input: Value) -> Result<Value, Error> {
    let id = meeting_path(&crate::input::<Meeting>(input)?.meeting)?;
    let found = cx.json(cx.get(&format!("{API}/meetings/{id}"))?).await?;
    let agenda = found["agenda"].as_str().unwrap_or_default();
    Ok(json!({
        "id": found["id"],
        "uuid": found["uuid"],
        "topic": found["topic"],
        "agenda": clip(agenda),
        "status": found["status"],
        "start": found["start_time"],
        "minutes": found["duration"],
        "timezone": found["timezone"],
        "host": found["host_email"],
        "join_url": found["join_url"],
        "recurring": found["recurrence"].is_object(),
        "waiting_room": found["settings"]["waiting_room"],
        "auto_recording": found["settings"]["auto_recording"],
    }))
}

async fn recordings(cx: &Context, input: Value) -> Result<Value, Error> {
    let wanted: Recordings = crate::input(input)?;
    let today = now() / DAY;
    let from = wanted.from.unwrap_or_else(|| date(today - 7));
    let to = wanted.to.unwrap_or_else(|| date(today));
    let size = wanted.limit.unwrap_or(30).clamp(1, 300).to_string();
    let mut query = vec![("from", from.as_str()), ("to", &to), ("page_size", &size)];
    if let Some(next) = &wanted.next {
        query.push(("next_page_token", next));
    }
    let path = format!("{API}/users/{}/recordings", user(cx)?);
    let found = cx.json(cx.get(&path)?.query(&query)).await?;
    let shown: Vec<Value> = found["meetings"]
        .as_array()
        .into_iter()
        .flatten()
        .map(recorded_meeting)
        .collect();
    Ok(json!({"recordings": shown, "next": next(&found)}))
}

fn recorded_meeting(item: &Value) -> Value {
    let files: Vec<Value> = item["recording_files"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|file| {
            json!({
                "id": file["id"],
                "type": file["file_type"],
                "recording": file["recording_type"],
                "size": file["file_size"],
                "status": file["status"],
            })
        })
        .collect();
    json!({
        "uuid": item["uuid"],
        "id": item["id"],
        "topic": item["topic"],
        "start": item["start_time"],
        "minutes": item["duration"],
        "files": files,
    })
}

/// Transcripts, captions and chat come back as text; other files go to the workspace.
async fn download(cx: &Context, input: Value) -> Result<Value, Error> {
    let wanted: Download = crate::input(input)?;
    let path = format!(
        "{API}/meetings/{}/recordings",
        meeting_path(&wanted.meeting)?
    );
    let found = cx.json(cx.get(&path)?).await?;
    let file = found["recording_files"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|file| file["id"] == wanted.file.as_str())
        .ok_or_else(|| Error::NotFound("no recording file with that id in the meeting".into()))?;
    let url = file["download_url"]
        .as_str()
        .ok_or_else(|| Error::NotFound("the file has no download address yet".into()))?;
    let (bytes, kind) = fetch(cx, url).await?;
    let extension = file["file_extension"]
        .as_str()
        .unwrap_or("bin")
        .to_ascii_lowercase();
    let recording = file["recording_type"].as_str().unwrap_or("recording");
    let name = format!("{recording}.{extension}");
    match (extension.as_str(), String::from_utf8(bytes)) {
        ("vtt" | "txt" | "json" | "csv", Ok(text)) => {
            Ok(json!({"name": name, "text": clip(&text)}))
        }
        (_, Ok(text)) => Ok(crate::file(&name, kind.as_deref(), text.as_bytes())),
        (_, Err(binary)) => Ok(crate::file(&name, kind.as_deref(), binary.as_bytes())),
    }
}

/// A download address with the token, which only ever goes to a Zoom host. Zoom answers with a
/// redirect to a signed address, followed without the token.
async fn fetch(cx: &Context, address: &str) -> Result<(Vec<u8>, Option<String>), Error> {
    let mut url = Url::parse(address)
        .ok()
        .filter(zoom_host)
        .ok_or_else(|| Error::Rejected("the download address is not on zoom.us".into()))?;
    let mut request = cx.get(url.as_str())?;
    for _ in 0..=REDIRECTS {
        let retry = request.try_clone();
        let response = request
            .send()
            .await
            .map_err(|_| Error::Unavailable("could not reach Zoom".into()))?;
        let status = response.status();
        if status.is_success() {
            return body(response).await;
        }
        if !status.is_redirection() {
            // Asked again through the context, which words the refusal.
            let retry =
                retry.ok_or_else(|| Error::Unavailable(format!("Zoom answered {status}")))?;
            return cx.bytes(retry).await;
        }
        url = response
            .headers()
            .get(header::LOCATION)
            .and_then(|location| location.to_str().ok())
            .and_then(|location| url.join(location).ok())
            .filter(|next| next.scheme() == "https")
            .ok_or_else(|| Error::Rejected("Zoom redirected somewhere unusable".into()))?;
        request = without_token(cx.get(url.as_str())?)?;
    }
    Err(Error::Rejected("Zoom redirected too many times".into()))
}

fn zoom_host(url: &Url) -> bool {
    let host = url.host_str().unwrap_or_default();
    url.scheme() == "https" && (host == "zoom.us" || host.ends_with(".zoom.us"))
}

fn without_token(request: RequestBuilder) -> Result<RequestBuilder, Error> {
    let (client, request) = request.build_split();
    let mut request =
        request.map_err(|_| Error::Invalid("the request could not be built".into()))?;
    request.headers_mut().remove(header::AUTHORIZATION);
    Ok(RequestBuilder::from_parts(client, request))
}

async fn body(mut response: Response) -> Result<(Vec<u8>, Option<String>), Error> {
    let kind = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|kind| kind.to_str().ok())
        .map(str::to_owned);
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| Error::Unavailable("the download was cut off".into()))?
    {
        if bytes.len() + chunk.len() > LARGEST {
            return Err(Error::Rejected("the file is larger than 25 MiB".into()));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok((bytes, kind))
}

/// Meetings from yesterday on whose recording files have all finished, by uuid; the cursor is
/// the JSON list of uuids already reported.
async fn poll(cx: &Context, cursor: Option<&str>) -> Result<Poll, Error> {
    let mut reported: Vec<String> = cursor
        .and_then(|cursor| serde_json::from_str(cursor).ok())
        .unwrap_or_default();
    let today = now() / DAY;
    let (from, to, size) = (date(today - 1), date(today), POLLED.to_string());
    let path = format!("{API}/users/{}/recordings", user(cx)?);
    let request =
        cx.get(&path)?
            .query(&[("from", from.as_str()), ("to", &to), ("page_size", &size)]);
    let found = cx.json(request).await?;
    let mut events = Vec::new();
    for item in found["meetings"].as_array().into_iter().flatten() {
        let Some(uuid) = item["uuid"].as_str() else {
            continue;
        };
        if reported.iter().any(|old| old == uuid) || !finished(item) {
            continue;
        }
        reported.push(uuid.into());
        let topic = item["topic"].as_str().unwrap_or("A meeting");
        events.push(Event {
            kind: "recording.completed",
            id: uuid.into(),
            summary: format!("The recording of {topic} is ready"),
            data: recorded_meeting(item),
        });
    }
    let excess = reported.len().saturating_sub(REMEMBERED);
    reported.drain(..excess);
    Ok(Poll {
        events: if cursor.is_some() { events } else { Vec::new() },
        cursor: Some(json!(reported).to_string()),
    })
}

fn finished(item: &Value) -> bool {
    let files = item["recording_files"].as_array();
    files.is_some_and(|files| {
        !files.is_empty() && files.iter().all(|file| file["status"] == "completed")
    })
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// `yyyy-mm-dd` of a day counted from 1970-01-01 (Howard Hinnant's civil-from-days).
fn date(days: u64) -> String {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn clip(text: &str) -> String {
    match text.char_indices().nth(LONGEST) {
        Some((end, _)) => format!("{}\n…[cut short]", &text[..end]),
        None => text.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Credential,
        testing::{Exchange, Fixture},
    };

    fn cx(fixture: &Fixture) -> Context {
        let credential = Credential::Token {
            token: "zoom-access-token".into(),
        };
        Context::new(json!({"user": "jchill@example.com"}), credential)
            .with_origin(&fixture.origin())
    }

    fn recorded(name: &str) -> Value {
        let path = format!("{}/fixtures/zoom/{name}.json", env!("CARGO_MANIFEST_DIR"));
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
    }

    #[test]
    fn dates_and_meeting_ids_are_written_as_zoom_wants() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(20_397), "2025-11-05");
        assert_eq!(date(11_016), "2000-02-29");
        assert_eq!(meeting_path("858 6232 4512").unwrap(), "85862324512");
        assert_eq!(
            meeting_path("4444AAAiAAAAAiAiAiiAii==").unwrap(),
            "4444AAAiAAAAAiAiAiiAii%3D%3D"
        );
        assert_eq!(
            meeting_path("/ajXp112QmuoKj4854875==").unwrap(),
            "%252FajXp112QmuoKj4854875%253D%253D"
        );
        assert!(meeting_path("../users/me").is_err());
    }

    #[tokio::test]
    async fn list_meetings_asks_for_the_users_previous_meetings_and_pages() {
        let fixture = Fixture::start(vec![Exchange::new(
            "GET",
            "/api.zoom.us/v2/users/jchill%40example.com/meetings",
            200,
            recorded("meetings"),
        )])
        .await;
        let listed = Zoom
            .call(&cx(&fixture), "list_meetings", json!({"when": "previous", "limit": 10, "next": "IAfJX3jsOLW7w3dokmFl84zOa0MAVGyMEB2"}))
            .await
            .unwrap();
        assert_eq!(listed["meetings"][0]["topic"], "My Meeting");
        assert_eq!(listed["next"], "Tva2CuIdTgsv8wAnhyAdU3m06Y2HuLQtlh3");
        let seen = fixture.finish().await;
        assert_eq!(
            seen[0].target,
            "/api.zoom.us/v2/users/jchill%40example.com/meetings?type=previous_meetings&page_size=10&next_page_token=IAfJX3jsOLW7w3dokmFl84zOa0MAVGyMEB2"
        );
        assert_eq!(
            seen[0].header("authorization"),
            Some("Bearer zoom-access-token")
        );
    }

    #[tokio::test]
    async fn read_meeting_and_list_recordings_are_compact() {
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.zoom.us/v2/meetings/85746065432",
                200,
                recorded("meeting"),
            ),
            Exchange::new(
                "GET",
                "/api.zoom.us/v2/users/jchill%40example.com/recordings?from=2026-10-01&to=2026-10-31&page_size=30",
                200,
                recorded("recordings"),
            ),
        ])
        .await;
        let read = Zoom
            .call(
                &cx(&fixture),
                "read_meeting",
                json!({"meeting": "85746065432"}),
            )
            .await
            .unwrap();
        assert_eq!(read["agenda"], "Quarterly planning");
        assert_eq!(read["host"], "jchill@example.com");
        let listed = Zoom
            .call(
                &cx(&fixture),
                "list_recordings",
                json!({"from": "2026-10-01", "to": "2026-10-31"}),
            )
            .await
            .unwrap();
        assert_eq!(listed["recordings"][0]["files"][1]["type"], "TRANSCRIPT");
        assert_eq!(listed["next"], Value::Null);
        fixture.finish().await;
    }

    #[tokio::test]
    async fn a_transcript_is_fetched_with_the_token_then_from_the_signed_address_without_it() {
        let uuid = "4444AAAiAAAAAiAiAiiAii==";
        let fixture = Fixture::start(vec![
            Exchange::new(
                "GET",
                "/api.zoom.us/v2/meetings/4444AAAiAAAAAiAiAiiAii%3D%3D/recordings",
                200,
                recorded("recordings")["meetings"][0].clone(),
            ),
            Exchange::raw("GET", "/us02web.zoom.us/rec/download/transcript", 302, "").header(
                "location",
                "https://ssrweb.zoom.us/file/transcript.vtt?sig=abc",
            ),
            Exchange::raw(
                "GET",
                "/ssrweb.zoom.us/file/transcript.vtt?sig=abc",
                200,
                "WEBVTT\n\n1\n00:00:01.000 --> 00:00:03.000\nJill Chill: Hello everyone",
            ),
        ])
        .await;
        let input = json!({"meeting": uuid, "file": "a2f19f96-9294-4f51-8134-6f0eea108eb2"});
        let got = Zoom
            .call(&cx(&fixture), "download_recording", input)
            .await
            .unwrap();
        assert_eq!(got["name"], "audio_transcript.vtt");
        assert!(got["text"].as_str().unwrap().ends_with("Hello everyone"));
        let seen = fixture.finish().await;
        assert_eq!(
            seen[1].header("authorization"),
            Some("Bearer zoom-access-token")
        );
        assert_eq!(seen[2].header("authorization"), None);
    }

    #[tokio::test]
    async fn a_download_address_off_zoom_never_gets_the_token() {
        let mut meeting = recorded("recordings")["meetings"][0].clone();
        meeting["recording_files"][1]["download_url"] = json!("https://evil.example/steal");
        let fixture = Fixture::start(vec![Exchange::new(
            "GET",
            "/api.zoom.us/v2/meetings/85746065432/recordings",
            200,
            meeting,
        )])
        .await;
        let input =
            json!({"meeting": "85746065432", "file": "a2f19f96-9294-4f51-8134-6f0eea108eb2"});
        let refused = Zoom.call(&cx(&fixture), "download_recording", input).await;
        assert!(matches!(refused, Err(Error::Rejected(_))));
        assert_eq!(fixture.finish().await.len(), 1);
    }

    #[tokio::test]
    async fn refusals_map_to_errors_and_bad_input_never_reaches_zoom() {
        let path = "/api.zoom.us/v2/meetings/1";
        let statuses = [
            (
                401,
                json!({"code": 124, "message": "Invalid access token."}),
            ),
            (
                403,
                json!({"code": 4711, "message": "Invalid access token, does not contain scopes:[meeting:read:meeting:admin]."}),
            ),
            (
                404,
                json!({"code": 3001, "message": "Meeting does not exist: 1."}),
            ),
            (
                429,
                json!({"code": 429, "message": "You have reached the maximum per-second rate limit for this API."}),
            ),
        ];
        let exchanges = statuses
            .iter()
            .map(|(status, body)| {
                Exchange::new("GET", path, *status, body.clone()).header("retry-after", "300")
            })
            .collect();
        let fixture = Fixture::start(exchanges).await;
        let mut got = Vec::new();
        for _ in &statuses {
            let answer = Zoom
                .call(&cx(&fixture), "read_meeting", json!({"meeting": "1"}))
                .await;
            got.push(answer.unwrap_err());
        }
        assert_eq!(got[0], Error::Unauthorized);
        assert!(matches!(&got[1], Error::Forbidden(message) if message.contains("scopes")));
        assert_eq!(got[2], Error::NotFound("Meeting does not exist: 1.".into()));
        assert!(matches!(got[3], Error::Unavailable(_)));
        let unknown = Zoom
            .call(&cx(&fixture), "list_meetings", json!({"type": "live"}))
            .await;
        assert!(matches!(unknown, Err(Error::Invalid(_))));
        assert_eq!(fixture.finish().await.len(), statuses.len());
    }

    #[tokio::test]
    async fn poll_records_finished_recordings_then_reports_new_ones_once() {
        let today = now() / DAY;
        let path = format!(
            "/api.zoom.us/v2/users/jchill%40example.com/recordings?from={}&to={}&page_size=30",
            date(today - 1),
            date(today)
        );
        let first = recorded("recordings");
        let mut second = first.clone();
        let mut fresh = first["meetings"][0].clone();
        fresh["uuid"] = json!("NewMeetingUuid==");
        fresh["topic"] = json!("Retro");
        second["meetings"].as_array_mut().unwrap().push(fresh);
        let mut processing = first["meetings"][0].clone();
        processing["uuid"] = json!("StillProcessing==");
        processing["recording_files"][0]["status"] = json!("processing");
        second["meetings"].as_array_mut().unwrap().push(processing);
        let fixture = Fixture::start(vec![
            Exchange::new("GET", &path, 200, first),
            Exchange::new("GET", &path, 200, second.clone()),
            Exchange::new("GET", &path, 200, second),
        ])
        .await;
        let baseline = Zoom.poll(&cx(&fixture), None).await.unwrap();
        assert!(baseline.events.is_empty());
        let next = Zoom
            .poll(&cx(&fixture), baseline.cursor.as_deref())
            .await
            .unwrap();
        let ids: Vec<&str> = next.events.iter().map(|event| event.id.as_str()).collect();
        assert_eq!(ids, ["NewMeetingUuid=="]);
        assert_eq!(next.events[0].summary, "The recording of Retro is ready");
        let again = Zoom
            .poll(&cx(&fixture), next.cursor.as_deref())
            .await
            .unwrap();
        assert!(again.events.is_empty());
        fixture.finish().await;
    }
}
