//! Last validated observations, separate from desired intent and execution readiness.
use super::{
    AgentId, Agents,
    kubernetes::{AttemptId, Phase},
};
use crate::identity::{Actor, IdentityError};
use serde::Serialize;

#[derive(Serialize, sqlx::FromRow)]
pub struct AgentStatus {
    pub id: AgentId,
    pub desired_state: String,
    pub revision: i64,
    pub restart_generation: i64,
    pub pod_attempt: Option<AttemptId>,
    pub pod_epoch: Option<i64>,
    pub pod_target_generation: Option<i64>,
    pub resource_phase: Option<Phase>,
    pub kubernetes_phase: Option<String>,
    pub kubernetes_reason: Option<String>,
    pub kubernetes_ready: Option<bool>,
    pub deleting: Option<bool>,
    pub observed_at: Option<String>,
    pub observed_seconds_ago: Option<f64>,
    pub blocked_reason: Option<String>,
    pub terminal_evidence_retained: bool,
    pub drain_requested: bool,
    pub drain_acknowledged: bool,
    /// Identity admission eligibility only, never authority to execute a turn.
    pub runtime_available: bool,
    pub recovery_identity_live: bool,
    pub recovery_records_pending: bool,
    pub workspace_attempt: Option<AttemptId>,
    pub workspace_resource_phase: Option<Phase>,
    pub workspace_blocked_reason: Option<String>,
}
impl Agents {
    pub async fn status(&self, actor: &Actor, id: AgentId) -> Result<AgentStatus, IdentityError> {
        sqlx::query_as("SELECT g.id,g.desired_state,g.revision,g.restart_generation,p.id AS pod_attempt,p.epoch AS pod_epoch,p.target_generation AS pod_target_generation,p.phase AS resource_phase,p.kubernetes_phase,p.kubernetes_reason,p.kubernetes_ready,p.deleting,(to_jsonb(p.observed_at) #>> '{}') AS observed_at,extract(epoch FROM (clock_timestamp() - p.observed_at))::float8 AS observed_seconds_ago,p.blocked_reason,p.termination_proof IS NOT NULL AS terminal_evidence_retained,d.request_id IS NOT NULL AS drain_requested,d.acknowledged_at IS NOT NULL AS drain_acknowledged,COALESCE(s.active,false) AS runtime_available,COALESCE(s.recovery,false) AS recovery_identity_live,EXISTS(SELECT 1 FROM agent_journal_records j WHERE j.agent_id = g.id AND j.reconciled_at IS NULL) AS recovery_records_pending,w.id AS workspace_attempt,w.phase AS workspace_resource_phase,w.blocked_reason AS workspace_blocked_reason FROM agents g JOIN human_accounts a ON a.id = $1 AND a.active LEFT JOIN LATERAL (SELECT r.* FROM agent_resources r WHERE r.agent_id = g.id AND r.kind = 'pod' ORDER BY r.epoch DESC LIMIT 1) p ON true LEFT JOIN agent_resources w ON w.agent_id = g.id AND w.kind = 'workspace' LEFT JOIN agent_runtime_drains d ON d.attempt_id = p.id LEFT JOIN runtime_session_access s ON s.attempt_id = p.id WHERE g.id = $2 AND (a.role = 'administrator' OR EXISTS(SELECT 1 FROM effective_human_grants e WHERE e.actor_id = a.id AND e.scope_kind = 'agent' AND e.scope_id = g.id AND e.capability = 'agent.manage'))")
            .bind(actor.id).bind(id).fetch_optional(&self.pool).await?.ok_or(IdentityError::NotFound)
    }
}
