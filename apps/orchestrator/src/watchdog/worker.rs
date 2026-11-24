//! The watchdog's one worker. Every ten seconds it reads the tool calls and audit entries recorded
//! since its cursor; every five minutes it runs the rate rules over the last fifteen. It writes
//! only findings, its cursor and audit entries, so agents never wait on it.
use super::{
    Reach, platform,
    rules::{self, Hit},
    sandbox,
};
use crate::agents::AgentId;
use crate::identity::audit;
use k8s_openapi::api::core::v1::Pod;
use serde_json::{Value, json};
use sqlx::PgPool;
use std::{
    collections::{BTreeMap, HashMap, HashSet, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    time::{Duration, Instant},
};
use tokio::sync::watch;
use uuid::Uuid;

const EVERY: Duration = Duration::from_secs(10);
const RATES: Duration = Duration::from_secs(5 * 60);
const BATCH: i64 = 500;
/// Batches per tick, so catching up after downtime never holds the worker for long.
const BATCHES: usize = 20;
/// Ten-minute windows in a week.
const WEEK: i64 = 7 * 24 * 6;
/// The same call this many times in one reply is a loop.
const LOOP: u32 = 8;
/// A rate rule's condition holds for its whole window, so it counts at most once per window.
const WINDOW_SECONDS: i64 = 15 * 60;
const SHARED: &[&str] = &[
    "skill_create",
    "skill_update",
    "kb_create",
    "kb_write",
    "kb_edit",
];

pub async fn watch(pool: PgPool, reach: Reach, mut stop: watch::Receiver<bool>) {
    let mut memory = Memory::default();
    let mut rated: Option<Instant> = None;
    loop {
        memory.read(&pool).await;
        rate(&pool, &reach, &mut rated).await;
        tokio::select! {
            _ = stop.changed() => return,
            () = tokio::time::sleep(EVERY) => (),
        }
    }
}

/// The rate rules, the agents' traffic and the namespace's pods when they are due; a failure is
/// logged and retried next round.
async fn rate(pool: &PgPool, reach: &Reach, rated: &mut Option<Instant>) {
    if rated.is_some_and(|at| at.elapsed() < RATES) {
        return;
    }
    *rated = Some(Instant::now());
    logged("the rate rules", rates(pool).await);
    logged("the sandboxes and the namespace", around(pool, reach).await);
}

fn logged(what: &str, outcome: Result<(), sqlx::Error>) {
    if let Err(error) = outcome {
        tracing::warn!(?error, "watchdog could not read {what}");
    }
}

/// What the VMMs and the cluster report, when this installation can read them.
async fn around(pool: &PgPool, reach: &Reach) -> Result<(), sqlx::Error> {
    if let Some(probe) = &reach.traffic {
        traffic(pool, probe).await?;
    }
    if let Some(list) = &reach.pods
        && let Some((namespace, pods)) = list().await
    {
        namespace_pods(pool, &namespace, &pods).await?;
    }
    Ok(())
}

async fn traffic(pool: &PgPool, probe: &sandbox::Probe) -> Result<(), sqlx::Error> {
    let agents = sqlx::query!(
        r#"
        SELECT g.id,
               NOT EXISTS(
                   SELECT 1 FROM chat_turns t
                   WHERE t.conversation_id = g.id
                     AND (t.status = 'running' OR t.status_at > clock_timestamp() - interval '5 minutes')
               ) AND NOT EXISTS(
                   SELECT 1 FROM dream_runs d
                   WHERE d.agent_id = g.id
                     AND (d.status = 'running' OR d.ended_at > clock_timestamp() - interval '5 minutes')
               ) AS "idle!"
        FROM agents g
        WHERE g.desired_state = 'running'
        "#
    )
    .fetch_all(pool)
    .await?;
    let reads = agents.iter().map(|agent| probe(AgentId(agent.id)));
    let summaries = futures_util::future::join_all(reads).await;
    for (agent, summary) in agents.iter().zip(summaries) {
        let Some(summary) = summary else {
            continue;
        };
        for hit in sandbox::traffic(&summary, agent.idle) {
            open(pool, &hit, Some(agent.id), None, WINDOW_SECONDS).await?;
        }
    }
    Ok(())
}

/// Findings about pods repeat each pass while they hold, so they count at most hourly.
async fn namespace_pods(pool: &PgPool, namespace: &str, pods: &[Pod]) -> Result<(), sqlx::Error> {
    let recorded: HashMap<String, platform::Recorded> = sqlx::query!(
        "
        SELECT name, agent_id, manifest FROM agent_resources
        WHERE namespace = $1 AND kind = 'pod' AND phase <> 'stopped'
        ",
        namespace,
    )
    .fetch_all(pool)
    .await?
    .into_iter()
    .map(|row| {
        let record = platform::Recorded {
            agent: row.agent_id,
            manifest: row.manifest,
        };
        (row.name, record)
    })
    .collect();
    for pod in pods {
        for (hit, agent) in platform::pod(pod, &recorded) {
            open(pool, &hit, agent, None, 3600).await?;
        }
    }
    Ok(())
}

/// One read of everything recorded since the cursor and one run of the rate rules, with
/// nothing remembered from earlier reads.
pub async fn sweep(pool: &PgPool, reach: &Reach) -> Result<(), sqlx::Error> {
    Memory::default().tick(pool).await?;
    rates(pool).await?;
    around(pool, reach).await
}

/// What the event rules remember between calls. It starts empty with the process.
// shortcut: send history lives in memory, so after a restart the first burst is judged against
// none; persist the buckets if restarts turn out to cause false alarms.
#[derive(Default)]
struct Memory {
    /// Per agent, sends per ten-minute window.
    sends: HashMap<Uuid, BTreeMap<i64, u32>>,
    /// Per turn, how often each exact call was made, and when the turn was last seen.
    calls: HashMap<Uuid, (Instant, HashMap<u64, u32>)>,
}

struct Call {
    turn: Uuid,
    sequence: i64,
    agent: Uuid,
    at: i64,
    payload: Value,
}

impl Memory {
    /// One read of the records; a failure is logged and the cursor stays for the next round.
    async fn read(&mut self, pool: &PgPool) {
        if let Err(error) = self.tick(pool).await {
            tracing::warn!(?error, "watchdog could not read the records");
        }
    }

    async fn tick(&mut self, pool: &PgPool) -> Result<(), sqlx::Error> {
        let cursor = sqlx::query!("SELECT turn_event, audit FROM watchdog_cursor")
            .fetch_one(pool)
            .await?;
        let sends = send_tools(pool).await?;
        let mut position = cursor.turn_event;
        for _ in 0..BATCHES {
            let (calls, next, more) = ledger(pool, position).await?;
            for call in &calls {
                self.call(pool, &sends, call).await?;
            }
            position = next;
            if !more {
                break;
            }
        }
        let audited = changes(pool, cursor.audit).await?;
        sqlx::query!(
            "UPDATE watchdog_cursor SET turn_event = $1, audit = $2, read_at = clock_timestamp()",
            position,
            audited,
        )
        .execute(pool)
        .await?;
        self.calls
            .retain(|_, (seen, _)| seen.elapsed() < Duration::from_secs(3600));
        Ok(())
    }

    async fn call(
        &mut self,
        pool: &PgPool,
        sends: &HashSet<String>,
        call: &Call,
    ) -> Result<(), sqlx::Error> {
        let name = call.payload["name"].as_str().unwrap_or_default();
        let input = &call.payload["input"];
        let mut hits = Vec::new();
        if name == "bash" {
            hits.extend(input["command"].as_str().and_then(rules::command));
        } else if SHARED.contains(&name) {
            hits.extend(rules::shared(name, &text(input)));
        } else if sends.contains(name) {
            hits.extend(rules::secret_sent(name, &text(input)));
            hits.extend(self.burst(call, name));
            hits.extend(injected(pool, call, name).await?);
        }
        hits.extend(self.repeated(call, name, input));
        for hit in &hits {
            let quiet = if hit.rule == "send_burst" { 600 } else { 0 };
            open(pool, hit, Some(call.agent), Some(call.turn), quiet).await?;
        }
        Ok(())
    }

    fn burst(&mut self, call: &Call, tool: &str) -> Option<Hit> {
        let window = call.at / 600;
        let windows = self.sends.entry(call.agent).or_default();
        windows.retain(|earlier, _| *earlier > window - WEEK);
        let busiest = windows
            .range(window - WEEK..window)
            .map(|(_, sends)| *sends)
            .max()
            .unwrap_or(0);
        let sends = windows.entry(window).or_insert(0);
        *sends += 1;
        (*sends >= 10 && *sends >= 3 * busiest).then(|| rules::burst(tool, *sends, busiest))
    }

    fn repeated(&mut self, call: &Call, name: &str, input: &Value) -> Option<Hit> {
        let mut hasher = DefaultHasher::new();
        (name, input.to_string()).hash(&mut hasher);
        let (seen, counts) = self
            .calls
            .entry(call.turn)
            .or_insert_with(|| (Instant::now(), HashMap::new()));
        *seen = Instant::now();
        let times = counts.entry(hasher.finish()).or_insert(0);
        *times += 1;
        (*times == LOOP).then(|| rules::repeated(name, &input.to_string(), LOOP))
    }
}

/// Tool calls after `after`, the position to resume from, and whether more are waiting.
async fn ledger(pool: &PgPool, after: i64) -> Result<(Vec<Call>, i64, bool), sqlx::Error> {
    let head = sqlx::query_scalar!(r#"SELECT COALESCE(max(id), 0) AS "head!" FROM turn_events"#)
        .fetch_one(pool)
        .await?;
    let rows = sqlx::query!(
        r#"
        SELECT e.id, e.turn_id, e.sequence, t.conversation_id AS agent, e.payload,
               extract(epoch FROM e.created_at)::bigint AS "at!"
        FROM turn_events e
        JOIN chat_turns t ON t.id = e.turn_id
        WHERE e.id > $1 AND e.id <= $2 AND e.payload->>'kind' = 'tool_call'
        ORDER BY e.id
        LIMIT $3
        "#,
        after,
        head,
        BATCH,
    )
    .fetch_all(pool)
    .await?;
    let more = rows.len() as i64 == BATCH;
    let next = match rows.last() {
        Some(last) if more => last.id,
        _ => head.max(after),
    };
    let calls = rows
        .into_iter()
        .map(|row| Call {
            turn: row.turn_id,
            sequence: row.sequence,
            agent: row.agent,
            at: row.at,
            payload: row.payload,
        })
        .collect();
    Ok((calls, next, more))
}

/// Connection tools tagged `Send`: irreversible, or reaching people outside the installation.
async fn send_tools(pool: &PgPool) -> Result<HashSet<String>, sqlx::Error> {
    let connections = sqlx::query!("SELECT slug, connector FROM connections")
        .fetch_all(pool)
        .await?;
    Ok(connections
        .iter()
        .filter_map(|row| Some((row, cubebot_connectors::find(&row.connector)?)))
        .flat_map(|(row, connector)| {
            connector
                .manifest()
                .tools
                .iter()
                .filter(|tool| tool.risk == cubebot_connectors::Risk::Send)
                .map(|tool| format!("{}__{}", row.slug, tool.name))
        })
        .collect())
}

/// A send whose turn had already read a result telling the model to drop its instructions.
async fn injected(pool: &PgPool, call: &Call, tool: &str) -> Result<Option<Hit>, sqlx::Error> {
    let results = sqlx::query_scalar!(
        r#"
        SELECT payload->'outcome' AS "outcome!"
        FROM turn_events
        WHERE turn_id = $1 AND sequence < $2 AND payload->>'kind' = 'tool_result'
        "#,
        call.turn,
        call.sequence,
    )
    .fetch_all(pool)
    .await?;
    Ok(results
        .iter()
        .find_map(|outcome| rules::injected_send(tool, &text(outcome))))
}

/// Sensitive changes people made since `after`; returns the position to resume from.
async fn changes(pool: &PgPool, after: i64) -> Result<i64, sqlx::Error> {
    let head =
        sqlx::query_scalar!(r#"SELECT COALESCE(max(sequence), 0) AS "head!" FROM audit_log"#)
            .fetch_one(pool)
            .await?;
    let rows = sqlx::query!(
        r#"
        SELECT l.kind, l.detail, h.login AS "login?"
        FROM audit_log l
        LEFT JOIN human_accounts h ON h.id = l.actor_id
        WHERE l.sequence > $1 AND l.sequence <= $2
          AND l.kind IN ('oidc.disable', 'oidc.configure', 'backup.key_downloaded', 'account.create',
                         'access.grant', 'provider.configure', 'provider.forget', 'provider.sign_in',
                         'provider.sign_out')
        ORDER BY l.sequence
        "#,
        after,
        head,
    )
    .fetch_all(pool)
    .await?;
    for row in rows {
        if let Some(hit) = rules::platform_change(&row.kind, &row.detail, row.login.as_deref()) {
            open(pool, &hit, None, None, 0).await?;
        }
    }
    Ok(head.max(after))
}

/// The rules over the last fifteen minutes (and, for spend, the week before them).
async fn rates(pool: &PgPool) -> Result<(), sqlx::Error> {
    let vault = sqlx::query!(
        r#"
        SELECT agent_id AS "agent!",
               count(*) FILTER (WHERE kind = 'vault.fill') AS "fills!",
               count(*) FILTER (WHERE kind = 'vault.card_request') AS "cards!",
               count(*) FILTER (WHERE kind = 'login.request') AS "logins!"
        FROM audit_log
        WHERE kind IN ('vault.fill', 'vault.card_request', 'login.request')
          AND agent_id IS NOT NULL AND at > clock_timestamp() - interval '15 minutes'
        GROUP BY agent_id
        "#
    )
    .fetch_all(pool)
    .await?;
    for row in vault {
        if let Some(hit) = rules::vault_burst(row.fills, row.cards, row.logins) {
            open(pool, &hit, Some(row.agent), None, WINDOW_SECONDS).await?;
        }
    }
    let failures = sqlx::query!(
        r#"
        SELECT count(*) AS "failures!", mode() WITHIN GROUP (ORDER BY detail->>'login') AS login
        FROM audit_log
        WHERE kind = 'session.sign_in_failed' AND at > clock_timestamp() - interval '15 minutes'
        "#
    )
    .fetch_one(pool)
    .await?;
    if let Some(hit) = rules::sign_in_failures(failures.failures, failures.login.as_deref()) {
        open(pool, &hit, None, None, WINDOW_SECONDS).await?;
    }
    spend(pool).await
}

async fn spend(pool: &PgPool) -> Result<(), sqlx::Error> {
    let agents = sqlx::query!(
        r#"
        SELECT agent_id AS "agent!",
               COALESCE(sum(cost_usd) FILTER (WHERE created_at > clock_timestamp() - interval '1 hour'), 0) AS "hour!",
               COALESCE(sum(cost_usd) FILTER (WHERE created_at <= clock_timestamp() - interval '1 hour'), 0) / 167 AS "usual!"
        FROM usage_events
        WHERE agent_id IS NOT NULL AND created_at > clock_timestamp() - interval '7 days'
        GROUP BY agent_id
        -- A new agent has no week to compare with.
        HAVING min(created_at) < clock_timestamp() - interval '1 day'
        "#
    )
    .fetch_all(pool)
    .await?;
    for row in agents {
        if let Some(hit) = rules::spend_spike(row.hour, row.usual) {
            open(pool, &hit, Some(row.agent), None, WINDOW_SECONDS).await?;
        }
    }
    Ok(())
}

/// Opens a finding, or counts a repeat on the open one at most once per `quiet` seconds. A
/// muted rule opens nothing. Opening one is recorded in the audit log.
pub(crate) async fn open(
    pool: &PgPool,
    hit: &Hit,
    agent: Option<Uuid>,
    turn: Option<Uuid>,
    quiet: i64,
) -> Result<(), sqlx::Error> {
    let opened = sqlx::query_scalar!(
        r#"
        INSERT INTO watchdog_findings (rule, severity, agent_id, turn_id, summary, evidence)
        SELECT $1, $2, $3, $4, $5, $6
        WHERE NOT EXISTS(
            SELECT 1 FROM watchdog_mutes m WHERE m.rule = $1 AND (m.agent_id IS NULL OR m.agent_id = $3)
        )
        ON CONFLICT (rule, COALESCE(agent_id, '00000000-0000-0000-0000-000000000000')) WHERE dismissed_at IS NULL
        DO UPDATE SET count = watchdog_findings.count + 1, seen_at = clock_timestamp(),
            turn_id = COALESCE(EXCLUDED.turn_id, watchdog_findings.turn_id),
            summary = EXCLUDED.summary,
            evidence = COALESCE(EXCLUDED.evidence, watchdog_findings.evidence)
        WHERE watchdog_findings.seen_at < clock_timestamp() - make_interval(secs => $7)
        RETURNING count = 1 AS "opened!"
        "#,
        hit.rule,
        hit.severity,
        agent,
        turn,
        clip(&hit.summary, 280),
        hit.evidence.as_deref().map(|evidence| clip(evidence, 1024)),
        quiet as f64,
    )
    .fetch_optional(pool)
    .await?;
    if opened == Some(true) {
        let mut event = audit::Event::anonymous("watchdog.flag").detail(json!({
            "rule": hit.rule,
            "severity": hit.severity,
            "summary": hit.summary,
            "scope": scope(agent),
        }));
        if let Some(agent) = agent {
            event = event.on(agent);
        }
        audit::record(pool, &event).await?;
    }
    Ok(())
}

/// Whether a finding is about one agent or the whole installation, for the audit log's wording.
pub(super) fn scope(agent: Option<Uuid>) -> &'static str {
    if agent.is_some() {
        "agent"
    } else {
        "installation"
    }
}

/// Every string in a tool's input or result, one per line; screenshots are skipped.
fn text(value: &Value) -> String {
    fn collect(value: &Value, into: &mut Vec<String>) {
        match value {
            Value::String(text) => into.push(text.clone()),
            Value::Array(items) => items.iter().for_each(|item| collect(item, into)),
            Value::Object(fields) => fields
                .iter()
                .filter(|(key, _)| key.as_str() != "image")
                .for_each(|(_, field)| collect(field, into)),
            _ => {}
        }
    }
    let mut strings = Vec::new();
    collect(value, &mut strings);
    strings.join("\n")
}

/// At most `bytes` bytes, cut on a character boundary.
fn clip(text: &str, bytes: usize) -> String {
    let mut end = text.len().min(bytes);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(at: i64) -> Call {
        Call {
            turn: Uuid::nil(),
            sequence: 1,
            agent: Uuid::nil(),
            at,
            payload: Value::Null,
        }
    }

    #[test]
    fn a_burst_is_judged_against_the_agents_own_week() {
        let mut memory = Memory::default();
        let earlier: Vec<_> = (0..4)
            .map(|_| memory.burst(&call(0), "mail__send"))
            .collect();
        assert!(earlier.iter().all(Option::is_none));
        let later = 6000;
        let hits: Vec<_> = (0..12)
            .filter_map(|_| memory.burst(&call(later), "mail__send"))
            .collect();
        // The busiest earlier window had 4, so the twelfth send in this one trips (12 >= 3 * 4).
        assert_eq!(hits.len(), 1);
        assert!(
            hits[0].summary.starts_with("Sent 12 times"),
            "{}",
            hits[0].summary
        );
    }

    #[test]
    fn a_loop_trips_once_per_call() {
        let mut memory = Memory::default();
        let input = json!({"command": "ls"});
        let hits = (0..20)
            .filter_map(|_| memory.repeated(&call(0), "bash", &input))
            .count();
        assert_eq!(hits, 1);
        assert!(
            memory
                .repeated(&call(0), "bash", &json!({"command": "pwd"}))
                .is_none()
        );
    }

    #[test]
    fn text_skips_screenshots_and_clip_keeps_characters_whole() {
        let value =
            json!({"status": "screenshot", "output": "ok", "image": "aWdub3Jl", "list": ["a", 1]});
        let mut lines: Vec<_> = text(&value).lines().map(str::to_owned).collect();
        lines.sort();
        assert_eq!(lines, ["a", "ok", "screenshot"]);
        assert_eq!(clip("héllo", 2), "h");
    }
}
