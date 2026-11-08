import { api, ok } from './client'
import type { components } from './schema'

type Schemas = components['schemas']
export type LifecycleAction = Schemas['LifecycleAction']
export type AgentStatus = Schemas['AgentStatus']
export type DefinitionRevision = Schemas['DefinitionRevision']

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
  createAccount: (login: string, password: string, administrator: boolean) =>
    ok(
      api.POST('/api/identity/accounts', {
        body: { request_id: id(), login, password, administrator },
      }),
    ),
  suspend: (account_id: string, expected_revision: number) =>
    ok(
      api.POST('/api/identity/suspend', {
        body: { request_id: id(), id: account_id, expected_revision },
      }),
    ),
  invitations: () => ok(api.GET('/api/identity/invitations')),
  invite: (login: string) =>
    ok(
      api.POST('/api/identity/invitations', {
        body: { request_id: id(), login },
      }),
    ),
  revokeInvitation: (invitation: string, expected_revision: number) =>
    ok(
      api.POST('/api/identity/invitations/revoke', {
        body: { request_id: id(), id: invitation, expected_revision },
      }),
    ),
  groups: () => ok(api.GET('/api/identity/groups')),
  createGroup: (name: string) =>
    ok(api.POST('/api/identity/groups', { body: { request_id: id(), name } })),
  membership: (
    group: string,
    account_id: string,
    expected_revision: number,
    present: boolean,
  ) =>
    ok(
      api.POST('/api/identity/groups/membership', {
        body: {
          request_id: id(),
          group,
          account_id,
          expected_revision,
          present,
        },
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
