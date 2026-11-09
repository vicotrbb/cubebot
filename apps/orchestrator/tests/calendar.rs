use crate::{
    connections::{Console, Vendor},
    runtime_identity::http_server,
    support,
};
use cubebot_orchestrator::{
    agents::{AgentId, Agents, CreateAgent},
    calendar::{Calendar, CalendarRange, Window},
    connections::{ConnectError, Connections, Credentials, FinishSignIn, SignInStarted},
    identity::{Actor, Identity, IdentityError},
    work::{Work, routines},
};
use serde_json::{Value, json};
use uuid::Uuid;

const EVENTS: &str = "/www.googleapis.com/calendar/v3/calendars";

struct Setup {
    pool: sqlx::PgPool,
    admin: Actor,
    member: Actor,
    vendor: Vendor,
    connections: Connections,
    calendar: Calendar,
}

async fn setup() -> Setup {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let session = identity
        .login("admin", "administrator password")
        .await
        .unwrap();
    let admin = identity.authenticate(&session.token).await.unwrap();
    identity
        .create_account(&admin, "member", "member password", false, Uuid::new_v4())
        .await
        .unwrap();
    let session = identity.login("member", "member password").await.unwrap();
    let member = identity.authenticate(&session.token).await.unwrap();
    let vendor = Vendor::default();
    let origin = vendor.serve().await;
    let connections = Connections::new(pool.clone(), Credentials::new([7; 32]))
        .with_vendor_for_loopback_tests(&origin);
    let calendar = Calendar::new(pool.clone(), Some(connections.clone()));
    Setup {
        pool,
        admin,
        member,
        vendor,
        connections,
        calendar,
    }
}

/// A Google connection with Calendar switched on, signed in by `owner`.
async fn google(s: &Setup, owner: &Actor, slug: &str) -> Uuid {
    let start = serde_json::from_value(json!({
        "connector": "google", "slug": slug, "auth": "oauth2",
        "config": {"client_id": "client-1.apps.googleusercontent.com", "products": ["calendar"]},
        "secrets": {"client_secret": "GOCSPX-secret"},
    }))
    .unwrap();
    let started = s
        .connections
        .start_connection_sign_in(owner.id, start, None);
    let SignInStarted::Redirect { sign_in, url, .. } = started.await.unwrap() else {
        panic!("a redirect sign-in");
    };
    let url = reqwest::Url::parse(&url).unwrap();
    let (_, state) = url.query_pairs().find(|(key, _)| key == "state").unwrap();
    s.vendor.answer(
        "/oauth2.googleapis.com/token",
        200,
        json!({"access_token": "ya29.calendar", "refresh_token": "1//refresh", "expires_in": 3599}),
    );
    s.vendor.answer(
        "/openidconnect.googleapis.com/v1/userinfo",
        200,
        json!({"email": format!("{slug}@example.com")}),
    );
    let landed = FinishSignIn {
        url: Some(format!(
            "http://127.0.0.1:1457/callback?state={state}&code=c"
        )),
    };
    let finished = s
        .connections
        .finish_connection_sign_in(owner.id, sign_in, landed)
        .await;
    finished.unwrap().unwrap().id
}

/// The account lists a writable `work` calendar and a read-only `holidays` one.
fn calendars(s: &Setup) {
    s.vendor.answer(
        "/www.googleapis.com/calendar/v3/users/me/calendarList",
        200,
        json!({"items": [
            {"id": "work", "summary": "Work", "accessRole": "owner"},
            {"id": "holidays", "summary": "Holidays", "accessRole": "reader"},
        ]}),
    );
}

fn standup(id: &str, start: &str, end: &str) -> Value {
    json!({
        "id": id, "recurringEventId": "standup", "summary": "Standup", "status": "confirmed",
        "start": {"dateTime": start, "timeZone": "America/Sao_Paulo"},
        "end": {"dateTime": end, "timeZone": "America/Sao_Paulo"},
        "htmlLink": "https://calendar.google.com/event?eid=1",
    })
}

async fn agent(s: &Setup, name: &str) -> AgentId {
    let made = Agents::new(s.pool.clone())
        .create(
            &s.admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: name.into(),
                instructions: "Be brief".into(),
                avatar: None,
                voice: None,
            },
        )
        .await
        .unwrap();
    AgentId(made.id)
}

async fn routine(s: &Setup, agent: AgentId, name: &str, schedule: Option<&str>) -> Uuid {
    let input = serde_json::from_value(json!({
        "request_id": Uuid::new_v4(), "webhook": schedule.is_none(),
        "routine": {"name": name, "prompt": "Report", "command": null, "schedule": schedule,
            "time_zone": "America/New_York", "coalesce": false, "enabled": true},
    }))
    .unwrap();
    let work = Work::new(s.pool.clone());
    let made = work.create_routine(&s.admin, agent, &input).await;
    made.unwrap().routine.id
}

fn window(from: &str, to: &str) -> Window {
    Window {
        from: from.into(),
        to: to.into(),
        time_zone: Some("America/Sao_Paulo".into()),
    }
}

async fn range(s: &Setup, viewer: &Actor, from: &str, to: &str) -> CalendarRange {
    s.calendar.range(viewer, &window(from, to)).await.unwrap()
}

fn ids(range: &CalendarRange, kind: &str) -> Vec<String> {
    let items = range.items.iter().filter(|item| item.kind == kind);
    items.map(|item| item.id.clone()).collect()
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn the_calendar_shows_my_calendar_events_and_my_agents_routines() {
    let s = setup().await;
    let connection = google(&s, &s.admin, "work").await;
    let keeper = agent(&s, "Keeper").await;
    let standup_routine = routine(&s, keeper, "Morning standup", Some("0 9 * * 1-5")).await;
    calendars(&s);
    s.vendor.answer(
        &format!("{EVENTS}/work/events"),
        200,
        json!({"items": [
            standup("standup_20300108T120000Z", "2030-01-08T09:00:00-03:00", "2030-01-08T09:15:00-03:00"),
            {"id": "gone", "status": "cancelled", "start": {"date": "2030-01-09"}, "end": {"date": "2030-01-10"}},
            {"id": "trip", "summary": "Trip", "status": "confirmed", "start": {"date": "2030-01-10"}, "end": {"date": "2030-01-12"}, "location": "Lisbon"},
        ]}),
    );
    s.vendor.answer(
        &format!("{EVENTS}/holidays/events"),
        200,
        json!({"items": []}),
    );

    // Monday 2030-01-07 to Monday 2030-01-14.
    let found = range(&s, &s.admin, "2030-01-07T03:00:00Z", "2030-01-14T03:00:00Z").await;
    let work = format!("calendar:{connection}:work");
    let sources: Vec<(&str, &str, bool)> = found
        .sources
        .iter()
        .map(|source| (source.id.as_str(), source.name.as_str(), source.writable))
        .collect();
    let holidays = format!("calendar:{connection}:holidays");
    let keeper_source = format!("agent:{}", keeper.0);
    assert_eq!(
        sources,
        [
            (work.as_str(), "Work", true),
            (holidays.as_str(), "Holidays", false),
            (keeper_source.as_str(), "Keeper", true),
        ]
    );
    let events: Vec<_> = found
        .items
        .iter()
        .filter(|item| item.kind == "event")
        .collect();
    assert_eq!(events.len(), 2, "the cancelled instance is left out");
    let first = events[0];
    assert_eq!(
        (
            first.start.as_str(),
            first.end.as_deref(),
            first.all_day,
            first.editable
        ),
        (
            "2030-01-08T12:00:00Z",
            Some("2030-01-08T12:15:00Z"),
            false,
            true
        )
    );
    assert_eq!(first.id, format!("{work}:standup_20300108T120000Z"));
    let detail = first.event.as_ref().unwrap();
    assert_eq!(
        (
            detail.event.as_str(),
            detail.series.as_deref(),
            detail.calendar.as_str()
        ),
        ("standup_20300108T120000Z", Some("standup"), "work")
    );
    let trip = events[1];
    assert_eq!(
        (trip.start.as_str(), trip.end.as_deref(), trip.all_day),
        ("2030-01-10", Some("2030-01-12"), true)
    );
    let runs: Vec<_> = found
        .items
        .iter()
        .filter(|item| item.kind == "routine")
        .collect();
    let starts: Vec<&str> = runs.iter().map(|item| item.start.as_str()).collect();
    assert_eq!(
        starts,
        [
            "2030-01-07T14:00:00Z",
            "2030-01-08T14:00:00Z",
            "2030-01-09T14:00:00Z",
            "2030-01-10T14:00:00Z",
            "2030-01-11T14:00:00Z"
        ],
        "09:00 in New York on weekdays"
    );
    assert_eq!(runs[0].routine.as_ref().unwrap().routine, standup_routine);
    let order: Vec<&str> = found.items.iter().map(|item| item.start.as_str()).collect();
    assert!(order.is_sorted(), "{order:?}");
    let asked = s.vendor.seen(&format!("{EVENTS}/work/events"));
    assert_eq!(asked.len(), 1);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn another_persons_connection_and_agents_are_not_shown() {
    let s = setup().await;
    let theirs = google(&s, &s.member, "theirs").await;
    let other = agent(&s, "Their agent").await;
    routine(&s, other, "Their routine", Some("0 9 * * *")).await;
    sqlx::query("UPDATE agents SET created_by = $1 WHERE id = $2")
        .bind(s.member.id)
        .bind(other.0)
        .execute(&s.pool)
        .await
        .unwrap();

    let found = range(&s, &s.admin, "2030-01-07T03:00:00Z", "2030-01-14T03:00:00Z").await;
    assert!(found.sources.is_empty() && found.items.is_empty());
    assert!(s.vendor.seen("/www.googleapis.com").is_empty());
    let new = serde_json::from_value(
        json!({"connection": theirs, "calendar": "work", "title": "Mine now",
        "start": "2030-01-08T12:00:00Z", "end": "2030-01-08T13:00:00Z", "all_day": false}),
    )
    .unwrap();
    let refused = s.calendar.create_event(&s.admin, &new).await;
    assert!(matches!(
        refused,
        Err(ConnectError::Identity(IdentityError::NotFound))
    ));
    let remove = serde_json::from_value(
        json!({"connection": theirs, "calendar": "work", "event": "x", "scope": "one"}),
    )
    .unwrap();
    let refused = s.calendar.delete_event(&s.admin, &remove).await;
    assert!(matches!(
        refused,
        Err(ConnectError::Identity(IdentityError::NotFound))
    ));

    // Their owner sees them.
    calendars(&s);
    s.vendor
        .answer(&format!("{EVENTS}/work/events"), 200, json!({"items": []}));
    s.vendor.answer(
        &format!("{EVENTS}/holidays/events"),
        200,
        json!({"items": []}),
    );
    let found = range(
        &s,
        &s.member,
        "2030-01-07T03:00:00Z",
        "2030-01-14T03:00:00Z",
    )
    .await;
    assert_eq!(found.sources.len(), 3);
    assert_eq!(ids(&found, "routine").len(), 7);
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn webhook_only_and_event_triggered_routines_are_not_shown() {
    let s = setup().await;
    let connection = google(&s, &s.member, "feed").await;
    let keeper = agent(&s, "Keeper").await;
    routine(&s, keeper, "On webhook", None).await;
    let on_event = routine(&s, keeper, "On event", None).await;
    let both = routine(&s, keeper, "Daily and on event", Some("30 7 * * *")).await;
    let paused = routine(&s, keeper, "Paused", Some("0 8 * * *")).await;
    sqlx::query("UPDATE routines SET trigger_connection = $1, trigger_event = 'event.starting' WHERE id = ANY($2)")
        .bind(connection)
        .bind(vec![on_event, both])
        .execute(&s.pool)
        .await
        .unwrap();
    sqlx::query("UPDATE routines SET enabled = false WHERE id = $1")
        .bind(paused)
        .execute(&s.pool)
        .await
        .unwrap();

    let found = range(&s, &s.admin, "2030-01-07T03:00:00Z", "2030-01-08T03:00:00Z").await;
    let shown: Vec<Uuid> = found
        .items
        .iter()
        .map(|item| item.routine.as_ref().unwrap().routine)
        .collect();
    assert_eq!(
        shown,
        [both],
        "only the scheduled, enabled routine, by its schedule"
    );
    assert_eq!(found.items[0].start, "2030-01-07T12:30:00Z");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn past_scheduled_runs_appear_with_their_outcome() {
    let s = setup().await;
    let keeper = agent(&s, "Keeper").await;
    let daily = routine(&s, keeper, "Daily report", Some("0 9 * * *")).await;
    sqlx::query(
        "UPDATE routines SET next_at = clock_timestamp() - interval '1 minute' WHERE id = $1",
    )
    .bind(daily)
    .execute(&s.pool)
    .await
    .unwrap();
    assert_eq!(routines::tick(&s.pool).await.unwrap(), 1);
    let work = Work::new(s.pool.clone());
    let hand = json!({"request_id": Uuid::new_v4()});
    work.run_routine(
        &s.admin,
        keeper,
        daily,
        &serde_json::from_value(hand).unwrap(),
    )
    .await
    .unwrap();
    let turn: Uuid = sqlx::query_scalar(
        "UPDATE chat_turns SET status = 'complete', response = 'Done', completed_at = clock_timestamp()
         WHERE routine_id = $1 AND origin = 'schedule' RETURNING id",
    )
    .bind(daily)
    .fetch_one(&s.pool)
    .await
    .unwrap();

    let now = chrono::DateTime::from_timestamp(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
        0,
    )
    .unwrap();
    let from = (now - chrono::TimeDelta::hours(1)).to_rfc3339();
    let to = (now + chrono::TimeDelta::hours(1)).to_rfc3339();
    let found = range(&s, &s.admin, &from, &to).await;
    let runs: Vec<_> = found
        .items
        .iter()
        .filter(|item| item.kind == "run")
        .collect();
    assert_eq!(runs.len(), 1, "the run by hand is not a scheduled one");
    let detail = runs[0].routine.as_ref().unwrap();
    assert_eq!(
        (
            detail.outcome.as_deref(),
            detail.turn,
            detail.routine,
            runs[0].editable
        ),
        (Some("complete"), Some(turn), daily, false)
    );
    assert_eq!(runs[0].id, format!("run:{turn}"));
    assert!(runs[0].end.is_some());
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_failing_connection_reports_its_problem_and_the_rest_still_shows() {
    let s = setup().await;
    let lapsed = google(&s, &s.admin, "lapsed").await;
    let working = google(&s, &s.admin, "working").await;
    // The lapsed one's token expired and its refresh is refused.
    sqlx::query("UPDATE connections SET expires_at = clock_timestamp() WHERE id = $1")
        .bind(lapsed)
        .execute(&s.pool)
        .await
        .unwrap();
    s.vendor.answer(
        "/oauth2.googleapis.com/token",
        400,
        json!({"error": "invalid_grant"}),
    );
    calendars(&s);
    s.vendor.answer(
        &format!("{EVENTS}/work/events"),
        200,
        json!({"items": [standup("standup_20300108T120000Z", "2030-01-08T09:00:00-03:00", "2030-01-08T09:15:00-03:00")]}),
    );
    s.vendor.answer(
        &format!("{EVENTS}/holidays/events"),
        500,
        json!({"error": {"message": "backend error"}}),
    );

    let found = range(&s, &s.admin, "2030-01-07T03:00:00Z", "2030-01-14T03:00:00Z").await;
    let problems: Vec<(String, bool)> = found
        .sources
        .iter()
        .map(|source| (source.id.clone(), source.problem.is_some()))
        .collect();
    assert_eq!(
        problems,
        [
            (format!("calendar:{lapsed}"), true),
            (format!("calendar:{working}:work"), false),
            (format!("calendar:{working}:holidays"), true),
        ]
    );
    let lapsed_source = &found.sources[0];
    assert_eq!(lapsed_source.name, "lapsed@example.com");
    assert!(
        lapsed_source
            .problem
            .as_deref()
            .unwrap()
            .contains("sign in again")
    );
    assert_eq!(
        ids(&found, "event"),
        [format!("calendar:{working}:work:standup_20300108T120000Z")]
    );
    let status: String = sqlx::query_scalar("SELECT status FROM connections WHERE id = $1")
        .bind(lapsed)
        .fetch_one(&s.pool)
        .await
        .unwrap();
    assert_eq!(status, "sign_in_again");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_read_level_connection_is_read_only() {
    let s = setup().await;
    let connection = google(&s, &s.admin, "read").await;
    let level = serde_json::from_value(json!({"level": "read"})).unwrap();
    s.connections
        .update_connection(connection, level)
        .await
        .unwrap();
    calendars(&s);
    s.vendor.answer(
        &format!("{EVENTS}/work/events"),
        200,
        json!({"items": [standup("standup_20300108T120000Z", "2030-01-08T09:00:00-03:00", "2030-01-08T09:15:00-03:00")]}),
    );
    s.vendor.answer(
        &format!("{EVENTS}/holidays/events"),
        200,
        json!({"items": []}),
    );

    let found = range(&s, &s.admin, "2030-01-07T03:00:00Z", "2030-01-14T03:00:00Z").await;
    assert!(found.sources.iter().all(|source| !source.writable));
    assert!(found.items.iter().all(|item| !item.editable));
    let new = serde_json::from_value(
        json!({"connection": connection, "calendar": "work", "title": "Lunch",
        "start": "2030-01-08T15:00:00Z", "end": "2030-01-08T16:00:00Z", "all_day": false}),
    )
    .unwrap();
    let refused = s.calendar.create_event(&s.admin, &new).await;
    assert!(matches!(
        refused,
        Err(ConnectError::Identity(IdentityError::Forbidden))
    ));
    let change = serde_json::from_value(json!({"connection": connection, "calendar": "work",
        "event": "standup_20300108T120000Z", "scope": "one", "title": "Moved"}))
    .unwrap();
    let refused = s.calendar.update_event(&s.admin, &change).await;
    assert!(matches!(
        refused,
        Err(ConnectError::Identity(IdentityError::Forbidden))
    ));
    assert!(s.vendor.seen(&format!("{EVENTS}/work/events/")).is_empty());
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_range_over_62_days_is_refused() {
    let pool = support::database().await;
    let identity = Identity::new(pool.clone());
    identity
        .bootstrap("admin", "administrator password")
        .await
        .unwrap();
    let token = identity
        .login("admin", "administrator password")
        .await
        .unwrap()
        .token;
    let router = cubebot_orchestrator::api::configured_router_with_credentials(
        pool,
        None,
        None,
        Some(Credentials::new([7; 32])),
    )
    .await
    .unwrap();
    let (origin, _task) = http_server(router).await;
    let api = Console {
        http: openidconnect::reqwest::Client::new(),
        origin,
        token,
    };
    let path = |to: &str| format!("/api/calendar?from=2030-01-01T00:00:00Z&to={to}");
    let (status, _) = api.json("GET", &path("2030-03-05T00:00:01Z"), None).await;
    assert_eq!(status, 400, "62 days and a second");
    let (status, _) = api.json("GET", &path("2029-12-31T00:00:00Z"), None).await;
    assert_eq!(status, 400, "ends before it starts");
    let (status, body) = api.json("GET", &path("2030-03-04T00:00:00Z"), None).await;
    assert_eq!((status, body), (200, json!({"sources": [], "items": []})));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn editing_one_occurrence_routes_to_the_occurrence_and_all_to_the_series() {
    let s = setup().await;
    let connection = google(&s, &s.admin, "work").await;
    let instance = "standup_20300108T120000Z";
    let change = |scope: &str| {
        serde_json::from_value(json!({"connection": connection, "calendar": "work", "event": instance,
            "series": "standup", "scope": scope, "title": "Standup (moved)", "time_zone": "America/Sao_Paulo"}))
        .unwrap()
    };
    let changed = standup(
        instance,
        "2030-01-08T09:00:00-03:00",
        "2030-01-08T09:15:00-03:00",
    );
    s.vendor.answer(
        &format!("{EVENTS}/work/events/{instance}"),
        200,
        changed.clone(),
    );
    s.calendar
        .update_event(&s.admin, &change("one"))
        .await
        .unwrap();
    let patched = s
        .vendor
        .seen(&format!("{EVENTS}/work/events/{instance}?sendUpdates=none"));
    assert_eq!(patched.len(), 1);
    assert!(
        patched[0].contains(r#""summary":"Standup (moved)""#),
        "{}",
        patched[0]
    );

    // All: the connector reads the occurrence to find its series, then patches the series.
    s.vendor
        .answer(&format!("{EVENTS}/work/events/{instance}"), 200, changed);
    s.vendor.answer(
        &format!("{EVENTS}/work/events/standup"),
        200,
        json!({"id": "standup", "summary": "Standup (moved)"}),
    );
    s.calendar
        .update_event(&s.admin, &change("all"))
        .await
        .unwrap();
    let series = s
        .vendor
        .seen(&format!("{EVENTS}/work/events/standup?sendUpdates=none"));
    assert_eq!(series.len(), 1);

    let remove = |scope: &str| {
        serde_json::from_value(
            json!({"connection": connection, "calendar": "work", "event": instance,
            "series": "standup", "scope": scope}),
        )
        .unwrap()
    };
    s.vendor
        .answer(&format!("{EVENTS}/work/events/{instance}"), 200, json!({}));
    s.calendar
        .delete_event(&s.admin, &remove("one"))
        .await
        .unwrap();
    s.vendor
        .answer(&format!("{EVENTS}/work/events/standup"), 200, json!({}));
    s.calendar
        .delete_event(&s.admin, &remove("all"))
        .await
        .unwrap();
    assert_eq!(
        s.vendor
            .seen(&format!("{EVENTS}/work/events/{instance}?sendUpdates=none"))
            .len(),
        2,
        "the occurrence's patch and its own deletion"
    );
    assert_eq!(
        s.vendor
            .seen(&format!("{EVENTS}/work/events/standup?sendUpdates=none"))
            .len(),
        2,
        "the series' patch and its deletion"
    );
}
