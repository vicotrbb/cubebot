import { api, ok } from './client'
import type { components } from './schema'

type Schemas = components['schemas']
export type LifecycleAction = Schemas['LifecycleAction']
export type AgentStatus = Schemas['AgentStatus']
export type DefinitionRevision = Schemas['DefinitionRevision']

export type Role = Schemas['Role']
export type Policy = Schemas['Policy']
export type Owned = Schemas['Owned']
export type GroupSource = Schemas['GroupSource']

const id = () => crypto.randomUUID()

export const admin = {
  agentStatus: (agent: string) =>
    ok(api.GET('/api/agents/{id}/status', { params: { path: { id: agent } } })),
  agentRevisions: (agent: string) =>
    ok(
      api.GET('/api/agents/{id}/revisions', {
        params: { path: { id: agent } },
      }),
    ),
  lifecycle: (
    agent: string,
    action: LifecycleAction,
    expected_revision: number,
  ) =>
    ok(
      api.POST('/api/agents/{id}/lifecycle', {
        params: { path: { id: agent } },
        body: { request_id: id(), expected_revision, action },
      }),
    ),
  setAvatar: (
    agent: string,
    current: Omit<Schemas['UpdateAgent'], 'request_id' | 'avatar'>,
    avatar: Schemas['Avatar'],
  ) =>
    ok(
      api.POST('/api/agents/{id}/definition', {
        params: { path: { id: agent } },
        body: { ...current, request_id: id(), avatar },
      }),
    ),
  accounts: () => ok(api.GET('/api/identity/accounts')),
  createAccount: (login: string, password: string, role: Role) =>
    ok(
      api.POST('/api/identity/accounts', {
        body: { request_id: id(), login, password, role },
      }),
    ),
  changeRole: (account: string, role: Role, expected_revision: number) =>
    ok(
      api.POST('/api/identity/accounts/role', {
        body: { request_id: id(), id: account, role, expected_revision },
      }),
    ),
  reactivate: (account: string, expected_revision: number) =>
    ok(
      api.POST('/api/identity/reactivate', {
        body: { request_id: id(), id: account, expected_revision },
      }),
    ),
  resetTwoFactor: (account: string) =>
    ok(
      api.POST('/api/identity/accounts/two-factor-reset', {
        body: { request_id: id(), id: account },
      }),
    ),
  passwordReset: (account: string) =>
    ok(
      api.POST('/api/identity/accounts/password-reset', {
        body: { request_id: id(), id: account },
      }),
    ),
  transfer: (kind: Owned, resource: string, to: string) =>
    ok(
      api.POST('/api/identity/ownership/transfer', {
        body: { request_id: id(), kind, id: resource, to },
      }),
    ),
  transferAll: (from: string, to: string) =>
    ok(
      api.POST('/api/identity/ownership/transfer-all', {
        body: { request_id: id(), from, to },
      }),
    ),
  handOver: (to: string) =>
    ok(
      api.POST('/api/identity/installation/hand-over', {
        body: { request_id: id(), to },
      }),
    ),
  policy: () => ok(api.GET('/api/identity/policy')),
  savePolicy: (policy: Policy) =>
    ok(api.PUT('/api/identity/policy', { body: { request_id: id(), policy } })),
  inventory: () => ok(api.GET('/api/identity/inventory')),
  spend: (days = 30) =>
    ok(api.GET('/api/identity/spend', { params: { query: { days } } })),
  directory: () => ok(api.GET('/api/identity/directory')),
  resourceAccess: (resource: string) =>
    ok(
      api.GET('/api/identity/access/resources/{id}', {
        params: { path: { id: resource } },
      }),
    ),
  google: () => ok(api.GET('/api/identity/google')),
  saveGoogle: (input: Omit<Schemas['SaveGoogle'], 'request_id'>) =>
    ok(
      api.PUT('/api/identity/google', { body: { ...input, request_id: id() } }),
    ),
  syncGoogle: () => ok(api.POST('/api/identity/google/sync')),
  workspaceGroups: () => ok(api.GET('/api/identity/google/groups')),
  suspend: (account_id: string, expected_revision: number) =>
    ok(
      api.POST('/api/identity/suspend', {
        body: { request_id: id(), id: account_id, expected_revision },
      }),
    ),
  invitations: () => ok(api.GET('/api/identity/invitations')),
  invite: (login: string, role: Role = 'member') =>
    ok(
      api.POST('/api/identity/invitations', {
        body: { request_id: id(), login, role },
      }),
    ),
  revokeInvitation: (invitation: string, expected_revision: number) =>
    ok(
      api.POST('/api/identity/invitations/revoke', {
        body: { request_id: id(), id: invitation, expected_revision },
      }),
    ),
  groups: () => ok(api.GET('/api/identity/groups')),
  createGroup: (
    name: string,
    linked?: { source: GroupSource; external_id: string },
  ) =>
    ok(
      api.POST('/api/identity/groups', {
        body: { request_id: id(), name, ...linked },
      }),
    ),
  membership: (
    group: string,
    account_id: string,
    expected_revision: number,
    present: boolean,
    manager?: boolean,
  ) =>
    ok(
      api.POST('/api/identity/groups/membership', {
        body: {
          request_id: id(),
          group,
          account_id,
          expected_revision,
          present,
          ...(manager === undefined ? {} : { manager }),
        },
      }),
    ),
  groupRole: (group: string, role: Role | null, expected_revision: number) =>
    ok(
      api.POST('/api/identity/groups/role', {
        body: { request_id: id(), group, role, expected_revision },
      }),
    ),
  deleteGroup: (group: string, expected_revision: number) =>
    ok(
      api.POST('/api/identity/groups/delete', {
        body: { request_id: id(), group, expected_revision },
      }),
    ),
  grants: () => ok(api.GET('/api/identity/access/grants')),
  grant: (
    recipient: Schemas['Recipient'],
    scope: Schemas['Scope'],
    capability: string,
  ) =>
    ok(
      api.POST('/api/identity/access/grants', {
        body: {
          request_id: id(),
          recipient,
          scope,
          capability: capability as Schemas['Capability'],
        },
      }),
    ),
  revokeGrant: (grant: string, expected_revision: number) =>
    ok(
      api.POST('/api/identity/access/revoke', {
        body: { request_id: id(), id: grant, expected_revision },
      }),
    ),
  explain: (scope: { kind: string; id: string }, capability: string) =>
    ok(
      api.POST('/api/identity/access/explain', {
        body: { scope, capability } as Schemas['Explain'],
      }),
    ),
  events: (after = 0) =>
    ok(
      api.GET('/api/identity/access/events', {
        params: { query: { after } },
      }),
    ),
  bindings: () => ok(api.GET('/api/identity/oidc/bindings')),
  revokeBinding: (binding: string, expected_revision: number) =>
    ok(
      api.POST('/api/identity/oidc/revoke', {
        body: { request_id: id(), id: binding, expected_revision },
      }),
    ),
  ready: () => api.GET('/api/health/ready').then(({ response }) => response),
}
