//! Recorded answers under `fixtures/google/` follow the examples in Google's API references.
use super::*;
use crate::{
    Credential,
    testing::{Exchange, Fixture},
};

const GMAIL: &str = "/gmail.googleapis.com/gmail/v1/users/me";
const PRIMARY: &str = "/www.googleapis.com/calendar/v3/calendars/primary/events";

fn cx(fixture: &Fixture, config: Value) -> Context {
    let credential = Credential::Token {
        token: "ya29.fake".into(),
    };
    Context::new(config, credential).with_origin(&fixture.origin())
}

fn recorded(name: &str) -> Value {
    let path = format!("{}/fixtures/google/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn get(path: &str, body: Value) -> Exchange {
    Exchange::new("GET", path, 200, body)
}

async fn call(fixture: &Fixture, tool: &str, input: Value) -> Value {
    Google
        .call(&cx(fixture, json!({})), tool, input)
        .await
        .unwrap()
}

#[test]
fn a_product_switched_off_offers_no_tools_and_asks_for_no_scope() {
    let config = json!({"products": ["gmail"]});
    let offered: Vec<&str> = MANIFEST
        .offered(&config, Level::Full, &[])
        .map(|tool| tool.name)
        .collect();
    assert!(offered.contains(&"gmail_send"));
    assert!(!offered.iter().any(|name| !name.starts_with("gmail_")));
    let Auth::OAuth2(oauth) = &MANIFEST.auth[0] else {
        panic!("Google signs in with OAuth")
    };
    assert_eq!(
        MANIFEST.scopes(oauth, &config),
        ["openid", "email", "https://mail.google.com/"]
    );
    let writing: Vec<&str> = MANIFEST
        .offered(&json!({}), Level::Write, &[])
        .map(|tool| tool.name)
        .collect();
    for sends in [
        "gmail_send",
        "calendar_invite",
        "calendar_respond",
        "drive_share",
    ] {
        assert!(!writing.contains(&sends), "{sends} reaches people");
    }
}

#[tokio::test]
async fn gmail_search_lists_ids_then_reads_each_ones_headers() {
    let fixture = Fixture::start(vec![
        get(&format!("{GMAIL}/messages"), recorded("messages")),
        get(
            &format!("{GMAIL}/messages/18c2f0a1b2c3d4e5"),
            recorded("metadata"),
        ),
        get(
            &format!("{GMAIL}/messages/18c2ef99aabbccdd"),
            recorded("metadata"),
        ),
    ])
    .await;
    let found = call(
        &fixture,
        "gmail_search",
        json!({"query": "from:ana", "limit": 5}),
    )
    .await;
    let first = &found["messages"][0];
    assert_eq!(first["from"], "Ana Souza <ana@example.org>");
    assert_eq!(first["subject"], "Q3 report");
    assert_eq!(first["thread"], "18c2f0a1b2c3d4e5");
    assert_eq!(found["messages"].as_array().unwrap().len(), 2);
    let seen = fixture.finish().await;
    assert!(seen[0].target.ends_with("?q=from%3Aana&maxResults=5"));
    assert!(seen[1].target.contains("format=metadata"));
}

#[tokio::test]
async fn gmail_read_thread_decodes_base64url_parts_and_lists_attachments() {
    let fixture = Fixture::start(vec![get(
        &format!("{GMAIL}/threads/18c2f0a1b2c3d4e5"),
        recorded("thread"),
    )])
    .await;
    let thread = call(
        &fixture,
        "gmail_read_thread",
        json!({"thread": "18c2f0a1b2c3d4e5"}),
    )
    .await;
    let [first, second] = thread["messages"].as_array().unwrap().as_slice() else {
        panic!("two messages")
    };
    assert_eq!(
        first["text"],
        "Hi Victor,\r\n\r\nThe Q3 report is attached — numbers look good?\r\n\r\nAna\r\n"
    );
    assert_eq!(first["attachments"][0]["attachment"], "ANGjdJ8report");
    assert_eq!(first["attachments"][0]["filename"], "q3-report.pdf");
    assert_eq!(
        second["text"], "Thanks Ana & team!\nLooks great.",
        "HTML only"
    );
    fixture.finish().await;
}

#[tokio::test]
async fn gmail_send_replies_in_the_thread_with_reply_headers() {
    let profile = json!({"emailAddress": "victor@example.com", "messagesTotal": 10, "threadsTotal": 8, "historyId": "1002"});
    let fixture = Fixture::start(vec![
        get(&format!("{GMAIL}/profile"), profile),
        get(
            &format!("{GMAIL}/messages/18c2f0a1b2c3d4e5"),
            recorded("metadata"),
        ),
        Exchange::new(
            "POST",
            &format!("{GMAIL}/messages/send"),
            200,
            json!({"id": "18c2f2", "threadId": "18c2f0a1b2c3d4e5", "labelIds": ["SENT"]}),
        ),
    ])
    .await;
    let input = json!({"to": ["Ana Souza <ana@example.org>"], "bcc": ["boss@example.com"], "text": "Looks great.", "reply_to": "18c2f0a1b2c3d4e5"});
    let sent = call(&fixture, "gmail_send", input).await;
    assert_eq!(sent["sent"]["thread"], "18c2f0a1b2c3d4e5");
    let posted = fixture.finish().await[2].json();
    assert_eq!(posted["threadId"], "18c2f0a1b2c3d4e5");
    let raw = decode(posted["raw"].as_str().unwrap()).unwrap();
    let raw = String::from_utf8(raw).unwrap();
    for header in [
        "From: victor@example.com\r\n",
        "To: \"Ana Souza\" <ana@example.org>\r\n",
        "Bcc: boss@example.com\r\n",
        "Subject: Re: Q3 report\r\n",
        "In-Reply-To: <CAF=abc123@mail.gmail.com>\r\n",
        "References: <CAF=abc123@mail.gmail.com>\r\n",
    ] {
        assert!(raw.contains(header), "{header:?} in {raw}");
    }
    assert!(raw.ends_with("\r\n\r\nLooks great."), "{raw}");
}

#[tokio::test]
async fn calendar_events_expands_recurring_events_in_start_order() {
    let fixture = Fixture::start(vec![get(PRIMARY, recorded("events"))]).await;
    let found = call(
        &fixture,
        "calendar_events",
        json!({"from": "2026-11-06T00:00:00Z"}),
    )
    .await;
    let review = &found["events"][0];
    assert_eq!(review["start"], "2026-11-06T10:00:00-03:00");
    assert_eq!(
        review["attendees"][0],
        json!({"email": "ana@example.org", "response": "accepted"})
    );
    assert_eq!(review["meet"], "https://meet.google.com/abc-defg-hij");
    assert_eq!(found["events"][1]["start"], "2026-11-15", "all day");
    let seen = fixture.finish().await;
    assert!(
        seen[0]
            .target
            .contains("singleEvents=true&orderBy=startTime")
    );
    assert!(seen[0].target.contains("timeMin=2026-11-06T00%3A00%3A00Z"));
}

#[tokio::test]
async fn calendar_create_event_notifies_no_one() {
    let made = recorded("events")["items"][1].clone();
    let fixture = Fixture::start(vec![Exchange::new(
        "POST",
        "/www.googleapis.com/calendar/v3/calendars/team@example.com/events",
        200,
        made,
    )])
    .await;
    let input = json!({"calendar": "team@example.com", "summary": "Holiday", "start": "2026-11-15", "end": "2026-11-16"});
    let event = call(&fixture, "calendar_create_event", input).await;
    assert_eq!(event["id"], "0a1b2c3d4e5f");
    let seen = &fixture.finish().await[0];
    assert!(seen.target.ends_with("?sendUpdates=none"));
    assert_eq!(seen.json()["start"], json!({"date": "2026-11-15"}));
}

#[tokio::test]
async fn drive_read_exports_a_google_doc_as_text() {
    let file = "/www.googleapis.com/drive/v3/files/1AbCdEfGhIjKlMnOpQrStUvWxYz0123456789";
    let fixture = Fixture::start(vec![
        get(file, recorded("drive_file")),
        Exchange::raw(
            "GET",
            &format!("{file}/export"),
            200,
            "Connections plan\r\nWave 1: Google.",
        )
        .header("content-type", "text/plain"),
    ])
    .await;
    let link = "https://docs.google.com/document/d/1AbCdEfGhIjKlMnOpQrStUvWxYz0123456789/edit";
    let read = call(&fixture, "drive_read", json!({"file": link})).await;
    assert_eq!(read["text"], "Connections plan\r\nWave 1: Google.");
    assert_eq!(read["name"], "Connections plan");
    let seen = fixture.finish().await;
    assert!(seen[1].target.ends_with("?mimeType=text%2Fplain"));
}

#[tokio::test]
async fn sheets_read_returns_rows_of_an_a1_range() {
    let fixture = Fixture::start(vec![get(
        "/sheets.googleapis.com/v4/spreadsheets/1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgvE2upms/values/Budget!A1:C3",
        recorded("values"),
    )])
    .await;
    let input = json!({"spreadsheet": "1BxiMVs0XRA5nFMdKvBdBZjgmUUqptlbs74OgvE2upms", "range": "Budget!A1:C3"});
    let read = call(&fixture, "sheets_read", input).await;
    assert_eq!(read["rows"][1], json!(["Laptop", "1200", "TRUE"]));
    assert_eq!(read["rows_left_out"], 0);
    fixture.finish().await;
}

#[tokio::test]
async fn contacts_search_warms_googles_cache_first() {
    let search = "/people.googleapis.com/v1/people:searchContacts";
    let fixture = Fixture::start(vec![
        get(search, json!({})),
        get(search, recorded("contacts")),
    ])
    .await;
    let found = call(&fixture, "contacts_search", json!({"query": "ana"})).await;
    assert_eq!(
        found["contacts"][0],
        json!({"id": "people/c8723456789012345678", "name": "Ana Souza", "emails": ["ana@example.org"], "phones": ["+55 11 91234-5678"]})
    );
    let seen = fixture.finish().await;
    assert!(seen[0].target.contains("query=&"), "{}", seen[0].target);
    assert!(seen[1].target.contains("query=ana&"), "{}", seen[1].target);
}

#[tokio::test]
async fn a_first_poll_starts_from_now_and_the_next_reports_new_inbox_mail() {
    let gmail_only = json!({"products": ["gmail"]});
    let profile = json!({"emailAddress": "victor@example.com", "historyId": "1000"});
    let fixture = Fixture::start(vec![
        get(&format!("{GMAIL}/profile"), profile),
        get(&format!("{GMAIL}/history"), recorded("history")),
        get(
            &format!("{GMAIL}/messages/18c2f0a1b2c3d4e5"),
            recorded("metadata"),
        ),
    ])
    .await;
    let cx = cx(&fixture, gmail_only);
    let first = Google.poll(&cx, None).await.unwrap();
    assert!(first.events.is_empty());
    assert_eq!(first.cursor.as_deref(), Some(r#"{"gmail":"1000"}"#));
    let next = Google.poll(&cx, first.cursor.as_deref()).await.unwrap();
    assert_eq!(next.cursor.as_deref(), Some(r#"{"gmail":"1002"}"#));
    let [event] = next.events.as_slice() else {
        panic!("one new message")
    };
    assert_eq!(event.kind, "mail.received");
    assert_eq!(event.id, "18c2f0a1b2c3d4e5");
    assert_eq!(event.summary, "Ana Souza <ana@example.org>: Q3 report");
    let seen = fixture.finish().await;
    assert!(
        seen[1]
            .target
            .contains("startHistoryId=1000&historyTypes=messageAdded&labelId=INBOX")
    );
}

#[tokio::test]
async fn an_event_starting_soon_is_reported_once_and_one_under_way_never() {
    let soon = rfc3339(now() + 5 * 60);
    let started = rfc3339(now() - 10 * 60);
    let items = json!({"items": [
        {"id": "under-way", "summary": "Standup", "start": {"dateTime": started}, "end": {"dateTime": soon}},
        {"id": "soon", "summary": "1:1 with Ana", "start": {"dateTime": soon}, "end": {"dateTime": soon}},
    ]});
    let fixture = Fixture::start(vec![
        get(PRIMARY, items.clone()),
        get(PRIMARY, items.clone()),
        get(PRIMARY, items),
    ])
    .await;
    let cx = cx(&fixture, json!({"products": ["calendar"]}));
    let baseline = r#"{"calendar":null}"#;
    let first = Google.poll(&cx, Some(baseline)).await.unwrap();
    assert!(first.events.is_empty(), "a new cursor reports nothing");
    let fresh = Google.poll(&cx, Some(r#"{"calendar":[]}"#)).await.unwrap();
    let [event] = fresh.events.as_slice() else {
        panic!("one event starting")
    };
    assert_eq!(event.summary, format!("1:1 with Ana starts at {soon}"));
    let again = Google.poll(&cx, fresh.cursor.as_deref()).await.unwrap();
    assert!(again.events.is_empty(), "already reported");
    fixture.finish().await;
}
