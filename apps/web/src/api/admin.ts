import { fetchJson, sendJson } from '../app/queries'
import type { Avatar } from '../components/Cube'

/* shortcut: typed by hand from the orchestrator's structs until these routes reach
   openapi.json (console contract item 1); switch to the generated client then. */

export type Receipt = { id: string; revision: number }
export type LifecycleAction = 'wake' | 'sleep' | 'restart' | 'retire'
export type AgentStatus = {
  id: string
  desired_state: string
  revision: number
  restart_generation: number
  pod_attempt: string | null
  pod_epoch: number | null
  resource_phase: string | null
  kubernetes_phase: string | null
  kubernetes_reason: string | null
  kubernetes_ready: boolean | null
  deleting: boolean | null
  observed_at: string | null
  observed_seconds_ago: number | null
  blocked_reason: string | null
  terminal_evidence_retained: boolean
  [extra: string]: unknown
}
export type DefinitionRevision = {
  revision: number
  instructions_revision: number
  name: string
  instructions: string
  idle_policy: string
  idle_delay_seconds: number
  dreaming_model: string | null
  task_lanes: number
  actor_id: string
  [extra: string]: unknown
}
export type AccountSummary = {
  id: string
  login: string
  administrator: boolean
  active: boolean
  revision: number
}
export type InvitationSummary = {
  id: string
  login: string
  revision: number
  status: string
  expires_at_unix: number
}
export type Team = {
  id: string
  name: string
  revision: number
  active: boolean
}
export type IdentityEvent = {
  sequence: number
  actor_id: string
  kind: string
  request_id: string
  resource_id: string
  revision: number
  input: unknown
  [extra: string]: unknown
}
export type Binding = {
  id: string
  account_id: string
  issuer: string
  subject: string
  active: boolean
  revision: number
}
export type Explanation = {
  allowed: boolean
  administrator: boolean
  grants: { id: string; team_id: string | null }[]
}

const id = () => crypto.randomUUID()

export const admin = {
  agentStatus: (agent: string) =>
    fetchJson<AgentStatus>(`/api/agents/${agent}/status`),
  agentRevisions: (agent: string) =>
    fetchJson<DefinitionRevision[]>(`/api/agents/${agent}/revisions`),
  lifecycle: (
    agent: string,
    action: LifecycleAction,
    expected_revision: number,
  ) =>
    sendJson<Receipt>(`/api/agents/${agent}/lifecycle`, 'POST', {
      request_id: id(),
      expected_revision,
      action,
    }),
  setAvatar: (
    agent: string,
    current: Record<string, unknown>,
    avatar: Avatar,
  ) =>
    sendJson<Receipt>(`/api/agents/${agent}/definition`, 'POST', {
      ...current,
      request_id: id(),
      avatar,
    }),
  accounts: () => fetchJson<AccountSummary[]>('/api/identity/accounts'),
  createAccount: (login: string, password: string, administrator: boolean) =>
    sendJson<Receipt>('/api/identity/accounts', 'POST', {
      request_id: id(),
      login,
      password,
      administrator,
    }),
  suspend: (account_id: string, expected_revision: number) =>
    sendJson<Receipt>('/api/identity/suspend', 'POST', {
      request_id: id(),
      account_id,
      expected_revision,
    }),
  invitations: () =>
    fetchJson<InvitationSummary[]>('/api/identity/invitations'),
  invite: (login: string) =>
    sendJson<{ id: string; token: string | null; revision: number }>(
      '/api/identity/invitations',
      'POST',
      { request_id: id(), login },
    ),
  revokeInvitation: (invitation: string, expected_revision: number) =>
    sendJson<Receipt>('/api/identity/invitations/revoke', 'POST', {
      request_id: id(),
      id: invitation,
      expected_revision,
    }),
  teams: () => fetchJson<Team[]>('/api/identity/teams'),
  createTeam: (name: string) =>
    sendJson<Receipt>('/api/identity/teams', 'POST', {
      request_id: id(),
      name,
    }),
  membership: (
    team: string,
    account_id: string,
    expected_revision: number,
    present: boolean,
  ) =>
    sendJson<Receipt>('/api/identity/teams/membership', 'POST', {
      request_id: id(),
      team,
      account_id,
      expected_revision,
      present,
    }),
  deleteTeam: (team: string, expected_revision: number) =>
    sendJson<Receipt>('/api/identity/teams/delete', 'POST', {
      request_id: id(),
      team,
      expected_revision,
    }),
  grant: (
    recipient: { kind: 'user' | 'team'; id: string },
    scope: { kind: 'agent' | 'room' | 'conversation'; id: string },
    capability: string,
  ) =>
    sendJson<Receipt>('/api/identity/access/grants', 'POST', {
      request_id: id(),
      recipient,
      scope,
      capability,
    }),
  revokeGrant: (grant: string, expected_revision: number) =>
    sendJson<Receipt>('/api/identity/access/revoke', 'POST', {
      request_id: id(),
      id: grant,
      expected_revision,
    }),
  explain: (scope: { kind: string; id: string }, capability: string) =>
    sendJson<Explanation>('/api/identity/access/explain', 'POST', {
      scope,
      capability,
    }),
  events: (after = 0) =>
    fetchJson<IdentityEvent[]>(`/api/identity/access/events?after=${after}`),
  bindings: () => fetchJson<Binding[]>('/api/identity/oidc/bindings'),
  revokeBinding: (binding: string, expected_revision: number) =>
    sendJson<Receipt>('/api/identity/oidc/revoke', 'POST', {
      request_id: id(),
      id: binding,
      expected_revision,
    }),
  ready: () =>
    fetch('/api/health/ready', {
      credentials: 'same-origin',
      cache: 'no-store',
    }),
}
