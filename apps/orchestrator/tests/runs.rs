use crate::{
    docker_sandbox::FAKE,
    memory_agent::{Model, Step, call, respond, text},
    registry_support,
    runtime_identity::{NativeChild, http_server, spawn_native},
};
use axum::Router;
use cubebot_orchestrator::{
    agents::{
        AgentId, Agents, CreateAgent, LifecycleAction, LifecycleCommand,
        runtime::RuntimeIdentity,
        sandbox::{Docker, DockerConfig, drive},
    },
    connections::{ConfigureProvider, Connections, Credentials},
    identity::{Actor, Identity},
    work::{CreateRoom, Delivery, RoomMessage, SendMessage, Work},
};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, sync::Arc, time::Duration};
use uuid::Uuid;

struct Member {
    id: Uuid,
    model: Arc<Model>,
    _process: NativeChild,
}

struct Crew {
    pool: sqlx::PgPool,
    work: Work,
    admin: Actor,
    room: Uuid,
    members: Vec<Member>,
    _servers: Vec<tokio::task::JoinHandle<()>>,
}

impl Crew {
    async fn new(names: &[&str]) -> Self {
        let (pool, _, _) = registry_support::setup().await;
        let identity = Identity::new(pool.clone());
        let session = identity
            .login("admin", "administrator password")
            .await
            .unwrap();
        let admin = identity.authenticate(&session.token).await.unwrap();
        provider(&pool, &admin).await;
        let runtime =
            RuntimeIdentity::docker(pool.clone()).with_credentials(Credentials::new([7; 32]));
        let (origin, server) =
            http_server(cubebot_orchestrator::api::runtime_router(runtime)).await;
        let mut servers = vec![server];
        let docker = docker(&pool);
        let mut members = Vec::new();
        for name in names {
            let id = started(&pool, &admin, name).await;
            drive(&docker.0, &pool, AgentId(id)).await.unwrap();
            let token = std::fs::read_to_string(
                docker
                    .1
                    .join("state")
                    .join(format!("cubebot-agent-{id}-1.token")),
            )
            .unwrap();
            let model = Arc::new(Model::default());
            let (provider, task) = http_server(
                Router::new()
                    .route("/responses", axum::routing::post(respond))
                    .with_state(model.clone()),
            )
            .await;
            servers.push(task);
            members.push(Member {
                id,
                model,
                _process: spawn_native(&origin, &provider, &token),
            });
        }
        let work = Work::new(pool.clone());
        let room = work
            .create_room(
                &admin,
                &CreateRoom {
                    request_id: Uuid::new_v4(),
                    name: "Launch".into(),
                    agents: members.iter().map(|member| member.id).collect(),
                },
            )
            .await
            .unwrap()
            .id;
        Self {
            pool,
            work,
            admin,
            room,
            members,
            _servers: servers,
        }
    }

    fn model(&self, index: usize) -> &Model {
        &self.members[index].model
    }

    async fn post(&self, text: &str) -> RoomMessage {
        self.work
            .post_room_message(
                &self.admin,
                self.room,
                &SendMessage {
                    request_id: Uuid::new_v4(),
                    message: text.into(),
                    attachments: Vec::new(),
                    app: None,
                },
            )
            .await
            .unwrap()
    }

    async fn messages(&self) -> Vec<RoomMessage> {
        self.work
            .room_messages(&self.admin, self.room, 0)
            .await
            .unwrap()
    }

    async fn delivery(&self, turn: Uuid, status: &str) -> Delivery {
        for _ in 0..300 {
            let found = self
                .messages()
                .await
                .into_iter()
                .flat_map(|message| message.deliveries)
                .find(|delivery| delivery.turn == turn);
            if let Some(found) = found.filter(|found| found.status == status) {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("turn {turn} never became {status}");
    }

    async fn turns_of(&self, agent: Uuid) -> Vec<(Uuid, String, Option<Uuid>, Option<Uuid>)> {
        sqlx::query!(
            "SELECT id, status, parent_turn_id, room_message_id FROM chat_turns WHERE conversation_id=$1 ORDER BY sequence",
            agent,
        )
        .fetch_all(&self.pool)
        .await
        .unwrap()
        .into_iter()
        .map(|row| (row.id, row.status, row.parent_turn_id, row.room_message_id))
        .collect()
    }
}

async fn provider(pool: &sqlx::PgPool, admin: &Actor) {
    Connections::new(pool.clone(), Credentials::new([7; 32]))
        .configure(
            admin,
            &ConfigureProvider {
                request_id: Uuid::new_v4(),
                expected_revision: 0,
                provider: cubebot_protocol::runtime::ProviderKind::Openai,
                model: "controlled-model".into(),
                context_window: Some(128_000),
                api_key: Some("controlled-key".into()),
                access: None,
                reasoning_effort: None,
            },
        )
        .await
        .unwrap();
}

fn docker(pool: &sqlx::PgPool) -> (Docker, std::path::PathBuf) {
    let root = std::env::temp_dir().join(format!("cubebot-runs-{}", Uuid::new_v4()));
    std::fs::create_dir(&root).unwrap();
    let program = root.join("docker");
    std::fs::write(&program, FAKE).unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = DockerConfig::parse(
        r#"{"image":"cubebot-sandbox:test","orchestrator_origin":"https://172.17.0.1:8443","control_ca_file":"/srv/ca.crt","runtime":{"kind":"kvm"}}"#,
    )
    .unwrap();
    let docker = Docker::new(pool.clone(), config).with_program(program.to_str().unwrap());
    (docker, root)
}

async fn started(pool: &sqlx::PgPool, admin: &Actor, name: &str) -> Uuid {
    let agents = Agents::new(pool.clone());
    let created = agents
        .create(
            admin,
            &CreateAgent {
                request_id: Uuid::new_v4(),
                name: name.into(),
                instructions: format!("{name} helps the launch"),
                avatar: None,
                voice: None,
            },
        )
        .await
        .unwrap();
    agents
        .lifecycle(
            admin,
            AgentId(created.id),
            &LifecycleCommand {
                request_id: Uuid::new_v4(),
                expected_revision: created.revision,
                action: LifecycleAction::Start,
            },
        )
        .await
        .unwrap();
    created.id
}

fn request_with(model: &Model, needle: &str) -> String {
    model
        .requests()
        .into_iter()
        .map(|request| request.to_string())
        .find(|request| request.contains(needle))
        .unwrap_or_else(|| panic!("no request contains {needle}"))
}

fn told(model: &Model, needle: &str) -> bool {
    model.requests().iter().any(|request: &Value| {
        request["input"].as_array().is_some_and(|items| {
            items.iter().any(|item| {
                item["type"] == "function_call_output"
                    && item["output"].to_string().contains(needle)
            })
        })
    })
}

fn handoffs(messages: &[RoomMessage], author: Uuid) -> Vec<&RoomMessage> {
    messages
        .iter()
        .filter(|message| message.author_agent_id == Some(author))
        .collect()
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn several_mentions_go_to_one_lead_whose_parts_show_in_the_room() {
    let crew = Crew::new(&["Ada", "Bee", "Cy"]).await;
    let (ada, bee, cy) = (crew.members[0].id, crew.members[1].id, crew.members[2].id);
    crew.model(0).turn(
        vec![
            Step::Reply(json!([
                text("Plan: Bee counts the rows, Cy checks the dates."),
                call(
                    "delegate",
                    json!({"agent":"Bee","message":"Count the rows in launch.csv"})
                ),
                call(
                    "delegate",
                    json!({"agent":"Cy","message":"Check the launch dates"})
                ),
            ])),
            Step::Reply(json!([text("Waiting on Bee and Cy.")])),
        ],
        json!([]),
    );
    crew.model(1)
        .turn(vec![Step::Gated(json!([text("42 rows")]))], json!([]));
    crew.model(2)
        .turn(vec![Step::Gated(json!([text("Dates hold")]))], json!([]));

    let asked = crew.post("@Ada @Bee @Cy prepare the launch numbers").await;
    let lead: Vec<Uuid> = asked.deliveries.iter().map(|d| d.agent_id).collect();
    assert_eq!(lead, [ada], "only the lead receives the request");
    let lead_turn = asked.deliveries[0].turn;
    crew.delivery(lead_turn, "delegated").await;
    tokio::time::timeout(Duration::from_secs(20), crew.model(1).gated.notified())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), crew.model(2).gated.notified())
        .await
        .unwrap();

    let first = request_with(crew.model(0), "prepare the launch numbers");
    assert!(first.contains("Your co-workers on it: Bee, Cy."), "{first}");
    assert!(
        first.contains("posted in the room"),
        "room wording for delegate"
    );
    let messages = crew.messages().await;
    let parts = handoffs(&messages, ada);
    assert_eq!(parts.len(), 2, "each hand-off is a visible room message");
    let mut helped = Vec::new();
    for part in &parts {
        let [child] = &part.deliveries[..] else {
            panic!("one turn per hand-off");
        };
        assert_eq!(child.parent_turn, Some(lead_turn));
        assert_eq!(child.status, "running");
        helped.push(child.agent_id);
    }
    helped.sort();
    let mut expected = vec![bee, cy];
    expected.sort();
    assert_eq!(helped, expected);
    let lead = messages
        .iter()
        .flat_map(|message| &message.deliveries)
        .find(|delivery| delivery.turn == lead_turn)
        .unwrap();
    assert!(
        lead.partial
            .as_deref()
            .unwrap_or_default()
            .contains("Plan: Bee counts the rows"),
        "the plan shows while the lead waits: {:?}",
        lead.partial
    );
    let bee_request = request_with(crew.model(1), "Count the rows in launch.csv");
    assert!(bee_request.contains("In room Launch, agent Ada delegated part of a request to you"));
    assert!(bee_request.contains("Lead: Ada, who writes the final answer."));
    let cy_request = request_with(crew.model(2), "Check the launch dates");
    // The delegate calls run concurrently, so only the later part is sure to see the other.
    assert!(
        bee_request.contains("- Cy, delegated by Ada: \\\"Check the launch dates\\\" (working)")
            || cy_request.contains(
                "- Bee, delegated by Ada: \\\"Count the rows in launch.csv\\\" (working)"
            ),
        "{bee_request}\n{cy_request}"
    );
    for member in [bee, cy] {
        let own = crew.work.history(&crew.admin, member, 0).await.unwrap();
        assert!(
            own.is_empty(),
            "room work stays out of the member's own conversation"
        );
    }
    crew.model(1).release.notify_one();
    crew.model(2).release.notify_one();
    crew.model(0).turn(
        vec![Step::Reply(json!([text("Rows 42; dates hold.")]))],
        json!([]),
    );
    crew.model(0).turn(
        vec![Step::Reply(json!([text("Rows 42; dates hold.")]))],
        json!([]),
    );
    let answered = crew.delivery(lead_turn, "complete").await;
    assert_eq!(answered.response.as_deref(), Some("Rows 42; dates hold."));
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_second_part_for_the_same_agent_is_refused() {
    let crew = Crew::new(&["Ada", "Bee"]).await;
    let (ada, bee) = (crew.members[0].id, crew.members[1].id);
    crew.model(0).turn(
        vec![
            Step::Reply(json!([
                call(
                    "delegate",
                    json!({"agent":"Bee","message":"Count the EU rows"})
                ),
                call(
                    "delegate",
                    json!({"agent":"Bee","message":"Count the US rows"})
                ),
            ])),
            Step::Reply(json!([text("Waiting on Bee.")])),
            Step::Reply(json!([text("EU has 12 rows.")])),
        ],
        json!([]),
    );
    crew.model(1)
        .turn(vec![Step::Reply(json!([text("12 rows")]))], json!([]));
    let asked = crew.post("@Ada @Bee count the rows").await;
    let answered = crew.delivery(asked.deliveries[0].turn, "complete").await;
    assert_eq!(answered.response.as_deref(), Some("EU has 12 rows."));
    assert!(told(
        crew.model(0),
        "Bee already has a part of this request"
    ));
    assert_eq!(crew.turns_of(bee).await.len(), 1, "Bee got one part");
    assert_eq!(
        handoffs(&crew.messages().await, ada).len(),
        1,
        "the refused part posts nothing"
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn the_turn_budget_stops_a_run_and_tells_the_lead_to_finish() {
    let crew = Crew::new(&["Ada", "Bee"]).await;
    let (ada, bee) = (crew.members[0].id, crew.members[1].id);
    crew.model(0).turn(
        vec![
            Step::Gated(json!([call(
                "delegate",
                json!({"agent":"Bee","message":"One more check"})
            )])),
            Step::Reply(json!([text("Finishing with what I have.")])),
        ],
        json!([]),
    );
    let asked = crew.post("@Ada @Bee audit the launch").await;
    let lead_turn = asked.deliveries[0].turn;
    tokio::time::timeout(Duration::from_secs(20), crew.model(0).gated.notified())
        .await
        .unwrap();
    for _ in 0..7 {
        sqlx::query!(
            "
            INSERT INTO chat_turns (conversation_id,actor_id,request_id,message,parent_turn_id,status,response,completed_at)
            VALUES ($1,$2,gen_random_uuid(),'earlier part',$3,'complete','done',clock_timestamp())
            ",
            ada,
            crew.admin.id,
            lead_turn,
        )
        .execute(&crew.pool)
        .await
        .unwrap();
    }
    crew.model(0).release.notify_one();
    let finished = crew.delivery(lead_turn, "complete").await;
    assert_eq!(
        finished.response.as_deref(),
        Some("Finishing with what I have.")
    );
    assert!(told(
        crew.model(0),
        "already used its 8 agent turns; finish with what you have"
    ));
    assert!(
        crew.turns_of(bee).await.is_empty(),
        "no turn starts past the budget"
    );
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn a_message_to_a_peer_already_working_steers_its_running_turn() {
    let crew = Crew::new(&["Ada", "Bee"]).await;
    let bee = crew.members[1].id;
    crew.model(0).turn(
        vec![
            Step::Reply(json!([call(
                "send_message",
                json!({"agent":"Bee","message":"Start the launch summary"})
            )])),
            Step::Gated(json!([call(
                "send_message",
                json!({"agent":"Bee","message":"Use metric units"})
            )])),
            Step::Reply(json!([text("Bee is writing it.")])),
        ],
        json!([]),
    );
    crew.model(1).turn(
        vec![
            Step::Gated(json!([call("search_memory", json!({"query":"launch"}))])),
            Step::Reply(json!([text("Summary in metric units")])),
        ],
        json!([]),
    );
    let asked = crew.post("@Ada @Bee write the launch summary").await;
    tokio::time::timeout(Duration::from_secs(20), crew.model(1).gated.notified())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), crew.model(0).gated.notified())
        .await
        .unwrap();
    crew.model(0).release.notify_one();
    crew.delivery(asked.deliveries[0].turn, "complete").await;
    let turns = crew.turns_of(bee).await;
    let [(running, status, _, _)] = &turns[..] else {
        panic!("the second message must not queue a turn: {turns:?}");
    };
    assert_eq!(status, "running");
    let steering: Vec<String> = sqlx::query_scalar!(
        "SELECT message FROM turn_steering WHERE turn_id=$1",
        running
    )
    .fetch_all(&crew.pool)
    .await
    .unwrap();
    assert_eq!(
        steering,
        ["Ada, who works on this request with you, wrote: Use metric units"]
    );
    tokio::time::sleep(Duration::from_millis(2500)).await;
    crew.model(1).release.notify_one();
    let done = crew.delivery(*running, "complete").await;
    assert_eq!(done.response.as_deref(), Some("Summary in metric units"));
    let steered = request_with(crew.model(1), "Use metric units");
    assert!(steered.contains("Steering about this request"), "{steered}");
}

#[tokio::test]
#[ignore = "requires isolated PostgreSQL; run just integration"]
async fn the_lead_resumes_after_a_failed_part_and_answers_with_what_it_has() {
    let crew = Crew::new(&["Ada", "Bee"]).await;
    crew.model(0).turn(
        vec![
            Step::Reply(json!([call(
                "delegate",
                json!({"agent":"Bee","message":"Fetch the supplier list"})
            )])),
            Step::Reply(json!([text("Waiting on Bee.")])),
            Step::Reply(json!([text(
                "Bee could not fetch the list, so it is missing."
            )])),
        ],
        json!([]),
    );
    crew.model(1).turn(vec![Step::Reject], json!([]));
    let asked = crew.post("@Ada @Bee list the suppliers").await;
    let lead_turn = asked.deliveries[0].turn;
    let answered = crew.delivery(lead_turn, "complete").await;
    assert_eq!(
        answered.response.as_deref(),
        Some("Bee could not fetch the list, so it is missing.")
    );
    let bee_turns = crew.turns_of(crew.members[1].id).await;
    assert_eq!(bee_turns[0].1, "failed");
    assert_eq!(bee_turns[0].2, Some(lead_turn));
    let resumed = request_with(crew.model(0), "You already delegated the following");
    assert!(resumed.contains(r#"\"status\":\"failed\""#), "{resumed}");
    assert!(
        resumed.contains("- Bee, delegated by Ada: \\\"Fetch the supplier list\\\" (failed)"),
        "{resumed}"
    );
}
